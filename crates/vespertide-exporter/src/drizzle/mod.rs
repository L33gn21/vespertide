mod enums;
mod render;
mod types;

use std::collections::HashSet;

use crate::orm::OrmExporter;
use vespertide_config::DrizzleConfig;
use vespertide_core::TableDef;
use vespertide_core::schema::column::{ColumnType, ComplexColumnType, EnumValues};
use vespertide_core::schema::constraint::TableConstraint;

use enums::{render_enum_decl, to_camel_case};
use render::{drizzle_default_needs_sql, extract_pk_info, render_relations_block, render_table};
use types::col_type_symbol;

pub struct DrizzleExporter;

impl OrmExporter for DrizzleExporter {
    fn render_entity(&self, table: &TableDef) -> Result<String, String> {
        Ok(render_entity(table))
    }

    fn render_entity_with_schema(
        &self,
        table: &TableDef,
        schema: &[TableDef],
    ) -> Result<String, String> {
        Ok(render_entity_with_schema(table, schema))
    }
}

/// Drizzle exporter with configuration support.
///
/// Assembles a complete `schema.ts` file from a full table list. Drizzle output
/// targets `drizzle-orm/pg-core` (PostgreSQL).
pub struct DrizzleExporterWithConfig<'a> {
    pub config: &'a DrizzleConfig,
}

impl<'a> DrizzleExporterWithConfig<'a> {
    pub fn new(config: &'a DrizzleConfig) -> Self {
        Self { config }
    }

    /// Render a complete `schema.ts` file for all tables.
    ///
    /// Output order: imports → enum declarations → table declarations →
    /// relations declarations.
    pub fn render_schema(&self, tables: &[TableDef]) -> String {
        // --- Collect all enums (globally deduped, in first-seen order). ---
        let mut seen_enums: HashSet<String> = HashSet::new();
        let mut enum_blocks: Vec<String> = Vec::new();
        for table in tables {
            for (name, values) in collect_table_enums(table) {
                if seen_enums.insert(name.to_string()) {
                    let const_name = to_camel_case(name);
                    enum_blocks.push(render_enum_decl(name, &const_name, values));
                }
            }
        }

        // pgEnum-derived type names are locally declared; exclude from pg-core imports.
        let enum_camel_names: HashSet<String> =
            seen_enums.iter().map(|n| to_camel_case(n)).collect();

        let mut used_col_types: HashSet<String> = HashSet::new();
        let mut needs_sql = false;
        let mut needs_relations = false;
        for table in tables {
            collect_used_types(
                table,
                tables,
                &enum_camel_names,
                &mut used_col_types,
                &mut needs_sql,
                &mut needs_relations,
            );
        }

        // --- Build pg-core import symbols. ---
        let mut pg_core_symbols: Vec<String> = vec!["pgTable".to_string()];
        if !enum_blocks.is_empty() {
            pg_core_symbols.push("pgEnum".to_string());
        }
        if tables.iter().any(has_composite_pk) {
            pg_core_symbols.push("primaryKey".to_string());
        }
        if tables.iter().any(has_unique_constraint) {
            pg_core_symbols.push("unique".to_string());
        }
        if tables.iter().any(has_index_constraint) {
            pg_core_symbols.push("index".to_string());
        }
        let mut sorted_col_types: Vec<String> = used_col_types.into_iter().collect();
        sorted_col_types.sort();
        pg_core_symbols.extend(sorted_col_types);
        pg_core_symbols.sort_by(|a, b| {
            // pgTable / pgEnum / constraint helpers first, then alphabetical.
            let rank = |s: &str| match s {
                "pgTable" => 0,
                "pgEnum" => 1,
                "primaryKey" => 2,
                "unique" => 3,
                "index" => 4,
                _ => 5,
            };
            rank(a).cmp(&rank(b)).then(a.cmp(b))
        });

        let mut parts: Vec<String> = Vec::new();
        parts.push(format!(
            "import {{ {} }} from \"drizzle-orm/pg-core\";",
            pg_core_symbols.join(", ")
        ));

        let mut drizzle_symbols: Vec<&str> = Vec::new();
        if needs_relations {
            drizzle_symbols.push("relations");
        }
        if needs_sql {
            drizzle_symbols.push("sql");
        }
        if !drizzle_symbols.is_empty() {
            parts.push(format!(
                "import {{ {} }} from \"drizzle-orm\";",
                drizzle_symbols.join(", ")
            ));
        }

        parts.extend(enum_blocks);

        for table in tables {
            parts.push(render_table(table, tables));
        }

        for table in tables {
            if let Some(block) = render_relations_block(table, tables) {
                parts.push(block);
            }
        }

        parts.join("\n\n") + "\n"
    }
}

fn has_composite_pk(t: &TableDef) -> bool {
    t.constraints
        .iter()
        .any(|c| matches!(c, TableConstraint::PrimaryKey { columns, .. } if columns.len() > 1))
}

fn has_unique_constraint(t: &TableDef) -> bool {
    t.constraints
        .iter()
        .any(|c| matches!(c, TableConstraint::Unique { columns, .. } if columns.len() > 1))
}

fn has_index_constraint(t: &TableDef) -> bool {
    t.constraints
        .iter()
        .any(|c| matches!(c, TableConstraint::Index { .. }))
}

fn collect_used_types(
    table: &TableDef,
    all_tables: &[TableDef],
    enum_camel_names: &HashSet<String>,
    used: &mut HashSet<String>,
    needs_sql: &mut bool,
    needs_relations: &mut bool,
) {
    let pk_info = extract_pk_info(&table.constraints);
    let pk_cols: HashSet<&str> = pk_info.columns.iter().map(String::as_str).collect();
    let is_composite_pk = pk_info.columns.len() > 1;

    for col in &table.columns {
        let in_pk = pk_cols.contains(col.name.as_str());
        let is_single_pk = in_pk && !is_composite_pk;
        let type_sym = col_type_symbol(&col.r#type, is_single_pk && pk_info.auto_increment);
        // Enum types are locally declared via pgEnum — not pg-core imports.
        if !enum_camel_names.contains(&type_sym) {
            used.insert(type_sym);
        }

        if let Some(ref default) = col.default
            && drizzle_default_needs_sql(&default.to_sql())
        {
            *needs_sql = true;
        }
    }

    let has_fk = table
        .constraints
        .iter()
        .any(|c| matches!(c, TableConstraint::ForeignKey { columns, .. } if columns.len() == 1));
    if has_fk || has_back_relations(table.name.as_str(), all_tables) {
        *needs_relations = true;
    }
}

fn has_back_relations(target_table: &str, schema: &[TableDef]) -> bool {
    schema.iter().any(|source| {
        source.constraints.iter().any(|c| {
            matches!(c, TableConstraint::ForeignKey { columns, ref_table, .. }
                if ref_table.as_str() == target_table && columns.len() == 1)
        })
    })
}

fn collect_table_enums(table: &TableDef) -> Vec<(&str, &EnumValues)> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for col in &table.columns {
        if let ColumnType::Complex(ComplexColumnType::Enum { name, values }) = &col.r#type
            && matches!(values, EnumValues::String(_))
            && seen.insert(name.as_str())
        {
            result.push((name.as_str(), values));
        }
    }
    result
}

/// Render enum blocks + table block without schema context (no relations).
pub fn render_entity(table: &TableDef) -> String {
    render_entity_with_schema(table, &[])
}

/// Render enum blocks + table block + relations with full schema context.
pub fn render_entity_with_schema(table: &TableDef, schema: &[TableDef]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (name, values) in collect_table_enums(table) {
        let const_name = to_camel_case(name);
        parts.push(render_enum_decl(name, &const_name, values));
    }
    parts.push(render_table(table, schema));
    if let Some(rel_block) = render_relations_block(table, schema) {
        parts.push(rel_block);
    }
    parts.join("\n\n")
}

/// Multi-table entry point: render every table (enum + table + relations
/// blocks) with full schema context and join them. Mirrors the other ORMs'
/// `export` so the cross-ORM test harness can dispatch Drizzle through a single
/// call. The import header lives in [`DrizzleExporterWithConfig::render_schema`].
pub fn export(schema: &[TableDef]) -> Result<String, String> {
    Ok(schema
        .iter()
        .map(|table| render_entity_with_schema(table, schema))
        .collect::<Vec<_>>()
        .join("\n\n"))
}

/// Test-only accessor for the internal `to_pascal_case` helper, mirroring the
/// other ORM backends so the cross-ORM consolidation test can exercise it
/// without making the helper generally public.
#[cfg(test)]
pub fn to_pascal_case_for_tests(s: &str) -> String {
    enums::to_pascal_case(s)
}
