use std::collections::{HashMap, HashSet};

use vespertide_core::TableDef;
use vespertide_core::schema::column::{ColumnType, ComplexColumnType, EnumValues, SimpleColumnType};
use vespertide_core::schema::constraint::TableConstraint;
use vespertide_core::schema::names::ColumnName;
use vespertide_core::schema::reference::ReferenceAction;

use super::enums::{infer_relation_field_name, to_camel_case, to_pascal_case};
use super::types::render_col_type_call;

// ─── PK helper ───────────────────────────────────────────────────────────────

pub(super) struct PkInfo {
    pub(super) columns: Vec<String>,
    pub(super) auto_increment: bool,
}

pub(super) fn extract_pk_info(constraints: &[TableConstraint]) -> PkInfo {
    for c in constraints {
        if let TableConstraint::PrimaryKey {
            auto_increment,
            columns,
            ..
        } = c
        {
            return PkInfo {
                columns: columns.iter().map(ToString::to_string).collect(),
                auto_increment: *auto_increment,
            };
        }
    }
    PkInfo {
        columns: Vec::new(),
        auto_increment: false,
    }
}

// ─── FK helper ───────────────────────────────────────────────────────────────

struct FkInfo<'a> {
    ref_table: &'a str,
    ref_cols: &'a [ColumnName],
    on_delete: Option<&'a ReferenceAction>,
    on_update: Option<&'a ReferenceAction>,
}

fn fk_by_col(table: &TableDef) -> HashMap<&str, FkInfo<'_>> {
    table
        .constraints
        .iter()
        .filter_map(|c| {
            if let TableConstraint::ForeignKey {
                columns,
                ref_table,
                ref_columns,
                on_delete,
                on_update,
                ..
            } = c
                && columns.len() == 1
            {
                Some((
                    columns[0].as_str(),
                    FkInfo {
                        ref_table: ref_table.as_str(),
                        ref_cols: ref_columns.as_slice(),
                        on_delete: on_delete.as_ref(),
                        on_update: on_update.as_ref(),
                    },
                ))
            } else {
                None
            }
        })
        .collect()
}

// ─── Table renderer ──────────────────────────────────────────────────────────

pub(super) fn render_table(table: &TableDef, _schema: &[TableDef]) -> String {
    let table_camel = to_camel_case(&table.name);
    let pk_info = extract_pk_info(&table.constraints);
    let pk_cols: HashSet<&str> = pk_info.columns.iter().map(String::as_str).collect();
    let is_composite_pk = pk_info.columns.len() > 1;

    let unique_single: HashMap<&str, Option<&str>> = table
        .constraints
        .iter()
        .filter_map(|c| {
            if let TableConstraint::Unique { name, columns, .. } = c
                && columns.len() == 1
            {
                Some((columns[0].as_str(), name.as_deref()))
            } else {
                None
            }
        })
        .collect();

    let fk_by_col = fk_by_col(table);

    // ── Column lines ────────────────────────────────────────────────────────
    let mut col_lines: Vec<String> = Vec::new();

    for col in &table.columns {
        let col_db = col.name.as_str();
        let col_js = to_camel_case(col_db);
        let in_pk = pk_cols.contains(col_db);
        let is_single_pk = in_pk && !is_composite_pk;
        let auto_inc = is_single_pk && pk_info.auto_increment;
        let is_unique_opt = unique_single.get(col_db).copied();

        let type_call = render_col_type_call(&col.r#type, col_db, auto_inc);

        let mut chain: Vec<String> = Vec::new();

        // .primaryKey() — single-column PK.
        if is_single_pk {
            chain.push(".primaryKey()".to_string());
        }

        // .notNull() — non-nullable, non-PK columns (serial/PK already imply NOT NULL).
        if !col.nullable && !is_single_pk {
            chain.push(".notNull()".to_string());
        }

        // .unique() — single-column unique (not the PK).
        if let Some(unique_name) = is_unique_opt
            && !is_single_pk
        {
            match unique_name {
                Some(n) => chain.push(format!(".unique(\"{n}\")")),
                None => chain.push(".unique()".to_string()),
            }
        }

        // .default(...) — skipped for auto-increment columns.
        if !auto_inc && let Some(ref default) = col.default {
            chain.push(drizzle_default_chain(&default.to_sql(), &col.r#type));
        }

        // .references(() => refTable.refCol, { ... })
        if let Some(fk) = fk_by_col.get(col_db) {
            let ref_table_camel = to_camel_case(fk.ref_table);
            let ref_col_camel = to_camel_case(fk.ref_cols.first().map_or("id", ColumnName::as_str));
            let mut ref_opts: Vec<String> = Vec::new();
            if let Some(od) = fk.on_delete {
                ref_opts.push(format!("onDelete: \"{}\"", reference_action_to_drizzle(od)));
            }
            if let Some(ou) = fk.on_update {
                ref_opts.push(format!("onUpdate: \"{}\"", reference_action_to_drizzle(ou)));
            }
            let ref_call = if ref_opts.is_empty() {
                format!(".references(() => {ref_table_camel}.{ref_col_camel})")
            } else {
                format!(
                    ".references(() => {ref_table_camel}.{ref_col_camel}, {{ {} }})",
                    ref_opts.join(", ")
                )
            };
            chain.push(ref_call);
        }

        col_lines.push(format!("  {col_js}: {type_call}{},", chain.join("")));
    }

    // ── Table-level constraints (callback) ────────────────────────────────────
    let mut constraint_lines: Vec<String> = Vec::new();

    // Composite PK.
    if is_composite_pk {
        let col_refs: Vec<String> = pk_info
            .columns
            .iter()
            .map(|c| format!("t.{}", to_camel_case(c)))
            .collect();
        constraint_lines.push(format!(
            "  pk: primaryKey({{ columns: [{}] }}),",
            col_refs.join(", ")
        ));
    }

    // Composite unique constraints.
    for c in &table.constraints {
        if let TableConstraint::Unique { name, columns, .. } = c
            && columns.len() > 1
        {
            let col_refs: Vec<String> = columns
                .iter()
                .map(|col| format!("t.{}", to_camel_case(col)))
                .collect();
            let unique_expr = match name {
                Some(n) => format!("unique(\"{n}\").on({})", col_refs.join(", ")),
                None => format!("unique().on({})", col_refs.join(", ")),
            };
            let key = name.as_deref().map_or_else(
                || {
                    let cols_key: String = columns.iter().map(|c| to_pascal_case(c)).collect();
                    format!("uq{cols_key}")
                },
                to_camel_case,
            );
            constraint_lines.push(format!("  {key}: {unique_expr},"));
        }
    }

    // Index constraints.
    for c in &table.constraints {
        if let TableConstraint::Index { name, columns } = c {
            let col_refs: Vec<String> = columns
                .iter()
                .map(|col| format!("t.{}", to_camel_case(col)))
                .collect();
            let index_expr = match name {
                Some(n) => format!("index(\"{n}\").on({})", col_refs.join(", ")),
                None => format!("index().on({})", col_refs.join(", ")),
            };
            let key = name.as_deref().map_or_else(
                || {
                    let cols_key: String = columns.iter().map(|c| to_pascal_case(c)).collect();
                    format!("idx{cols_key}")
                },
                to_camel_case,
            );
            constraint_lines.push(format!("  {key}: {index_expr},"));
        }
    }

    // ── Assemble pgTable call ─────────────────────────────────────────────────
    let mut lines: Vec<String> = Vec::new();

    if let Some(desc) = &table.description {
        for line in desc.lines() {
            lines.push(format!("// {line}"));
        }
    }

    lines.push(format!(
        "export const {table_camel} = pgTable(\"{}\", {{",
        table.name
    ));
    lines.extend(col_lines);
    if constraint_lines.is_empty() {
        lines.push("});".to_string());
    } else {
        lines.push("}, (t) => ({".to_string());
        lines.extend(constraint_lines);
        lines.push("}));".to_string());
    }

    lines.join("\n")
}

// ─── Relations renderer ──────────────────────────────────────────────────────

struct BackRelation {
    source_table: String,
    fk_col: String,
    is_one_to_one: bool,
    relation_name: Option<String>,
}

fn collect_back_relations(target_table: &str, schema: &[TableDef]) -> Vec<BackRelation> {
    let mut result = Vec::new();

    for source in schema {
        let fks_to_target: Vec<&str> = source
            .constraints
            .iter()
            .filter_map(|c| {
                if let TableConstraint::ForeignKey {
                    columns, ref_table, ..
                } = c
                    && ref_table.as_str() == target_table
                    && columns.len() == 1
                {
                    Some(columns[0].as_str())
                } else {
                    None
                }
            })
            .collect();

        if fks_to_target.is_empty() {
            continue;
        }

        let multi_fk = fks_to_target.len() > 1;
        let is_self_ref = source.name.as_str() == target_table;

        for fk_col in &fks_to_target {
            let is_unique = source.constraints.iter().any(|c| {
                matches!(c, TableConstraint::Unique { columns, .. }
                    if columns.len() == 1 && columns[0].as_str() == *fk_col)
            });

            let needs_name = multi_fk || is_self_ref;
            let relation_name = if needs_name {
                let rel_field = infer_relation_field_name(fk_col);
                Some(format!(
                    "{}{}",
                    to_pascal_case(&source.name),
                    to_pascal_case(&rel_field)
                ))
            } else {
                None
            };

            result.push(BackRelation {
                source_table: source.name.as_str().to_string(),
                fk_col: (*fk_col).to_string(),
                is_one_to_one: is_unique,
                relation_name,
            });
        }
    }

    result
}

/// Render a `relations(...)` export block, or `None` if the table has no relations.
pub(super) fn render_relations_block(table: &TableDef, schema: &[TableDef]) -> Option<String> {
    let table_camel = to_camel_case(&table.name);
    let fk_by_col = fk_by_col(table);

    // Count FKs per ref_table for disambiguation.
    let mut ref_table_fk_count: HashMap<&str, usize> = HashMap::new();
    for fk in fk_by_col.values() {
        *ref_table_fk_count.entry(fk.ref_table).or_default() += 1;
    }

    // Even with no external schema context (single-table render), self-referencing
    // FKs produce a back-relation that must be emitted so the forward `one(...)`
    // side's `relationName` has a matching reciprocal — otherwise Drizzle throws
    // "not enough information to infer relation" at runtime.
    let back_rels = if schema.is_empty() {
        collect_back_relations(&table.name, std::slice::from_ref(table))
    } else {
        collect_back_relations(&table.name, schema)
    };

    let mut rel_lines: Vec<String> = Vec::new();

    // Forward relations: one entry per FK column, in column order.
    for col in &table.columns {
        let col_name = col.name.as_str();
        if let Some(fk) = fk_by_col.get(col_name) {
            let col_camel = to_camel_case(col_name);
            let rel_field_raw = infer_relation_field_name(col_name);
            let mut rel_field_camel = to_camel_case(&rel_field_raw);
            // Disambiguate when the FK column has no `_id` suffix: the inferred
            // relation field name would otherwise collide with the column
            // property of the same name (Drizzle merges columns + relations in
            // query results).
            if rel_field_camel == col_camel {
                rel_field_camel = to_camel_case(&format!("{}_{}", rel_field_raw, fk.ref_table));
            }
            let ref_table_camel = to_camel_case(fk.ref_table);

            let multi_fk = ref_table_fk_count.get(fk.ref_table).copied().unwrap_or(0) > 1;
            let is_self_ref = fk.ref_table == table.name.as_str();
            let needs_name = multi_fk || is_self_ref;

            let ref_col = fk.ref_cols.first().map_or("id", ColumnName::as_str);
            let ref_col_camel = to_camel_case(ref_col);

            let mut one_opts: Vec<String> = vec![
                format!("fields: [{table_camel}.{col_camel}]"),
                format!("references: [{ref_table_camel}.{ref_col_camel}]"),
            ];
            if needs_name {
                let rel_name = format!(
                    "{}{}",
                    to_pascal_case(&table.name),
                    to_pascal_case(&rel_field_raw)
                );
                one_opts.push(format!("relationName: \"{rel_name}\""));
            }

            rel_lines.push(format!(
                "  {rel_field_camel}: one({ref_table_camel}, {{ {} }}),",
                one_opts.join(", ")
            ));
        }
    }

    // Back relations.
    for br in &back_rels {
        let source_camel = to_camel_case(&br.source_table);
        let field_name = if br.relation_name.is_some() {
            let rel_field = infer_relation_field_name(&br.fk_col);
            format!("{rel_field}_{}", br.source_table)
        } else {
            br.source_table.clone()
        };
        let field_name_camel = to_camel_case(&field_name);

        let opts_str = match &br.relation_name {
            Some(rel_name) => format!(", {{ relationName: \"{rel_name}\" }}"),
            None => String::new(),
        };

        if br.is_one_to_one {
            rel_lines.push(format!(
                "  {field_name_camel}: one({source_camel}{opts_str}),"
            ));
        } else {
            rel_lines.push(format!(
                "  {field_name_camel}: many({source_camel}{opts_str}),"
            ));
        }
    }

    if rel_lines.is_empty() {
        return None;
    }

    let mut lines: Vec<String> = vec![format!(
        "export const {table_camel}Relations = relations({table_camel}, ({{ one, many }}) => ({{"
    )];
    lines.extend(rel_lines);
    lines.push("}));".to_string());

    Some(lines.join("\n"))
}

// ─── Default value rendering ─────────────────────────────────────────────────

/// Whether a SQL default expression must be emitted via a `sql\`...\`` tagged
/// template rather than a literal `.default(...)` argument.
pub(super) fn drizzle_default_needs_sql(default_sql: &str) -> bool {
    if default_sql == "true" || default_sql == "false" {
        return false;
    }
    let lower = default_sql.to_lowercase();
    if lower.contains("now()") || lower.starts_with("current_timestamp") {
        return false;
    }
    if lower.contains("gen_random_uuid()")
        || lower.contains("uuid_generate_v4()")
        || lower.contains("newid()")
    {
        return false;
    }
    if default_sql.contains('(') {
        return true;
    }
    if default_sql.starts_with('\'') || default_sql.starts_with('"') {
        return false;
    }
    if default_sql.parse::<f64>().is_ok() {
        return false;
    }
    // Bare keyword fallback → sql``.
    true
}

fn drizzle_default_chain(default_sql: &str, col_type: &ColumnType) -> String {
    if default_sql == "true" {
        return ".default(true)".to_string();
    }
    if default_sql == "false" {
        return ".default(false)".to_string();
    }

    let lower = default_sql.to_lowercase();
    if lower.contains("now()") || lower.starts_with("current_timestamp") {
        return ".defaultNow()".to_string();
    }
    if lower.contains("gen_random_uuid()")
        || lower.contains("uuid_generate_v4()")
        || lower.contains("newid()")
    {
        return ".defaultRandom()".to_string();
    }

    // JSON object/array literal default for a json/jsonb column — must be a
    // valid Postgres jsonb literal, not a bare SQL fragment.
    if matches!(col_type, ColumnType::Simple(SimpleColumnType::Json)) {
        let trimmed = default_sql.trim();
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            let escaped = trimmed
                .replace('\\', "\\\\")
                .replace('`', "\\`")
                .replace('\'', "''");
            return format!(".default(sql`'{escaped}'::jsonb`)");
        }
    }

    // Any remaining function call → sql``.
    if default_sql.contains('(') {
        let escaped = default_sql.replace('`', "\\`");
        return format!(".default(sql`{escaped}`)");
    }

    // String literal with quotes — may be an enum value.
    if default_sql.starts_with('\'') || default_sql.starts_with('"') {
        let stripped = default_sql.trim_matches(|c| c == '\'' || c == '"');
        if let ColumnType::Complex(ComplexColumnType::Enum {
            values: EnumValues::String(variants),
            ..
        }) = col_type
            && variants.iter().any(|v| v.as_str() == stripped)
        {
            return format!(".default(\"{stripped}\")");
        }
        let escaped = stripped.replace('\\', "\\\\").replace('"', "\\\"");
        return format!(".default(\"{escaped}\")");
    }

    // Numeric.
    if default_sql.parse::<f64>().is_ok() {
        return format!(".default({default_sql})");
    }

    // Integer enum variant name → resolve to its numeric value.
    if let ColumnType::Complex(ComplexColumnType::Enum {
        values: EnumValues::Integer(variants),
        ..
    }) = col_type
        && let Some(variant) = variants.iter().find(|v| v.name == default_sql)
    {
        return format!(".default({})", variant.value);
    }

    // Fallback: bare keyword → sql``.
    let escaped = default_sql.replace('`', "\\`");
    format!(".default(sql`{escaped}`)")
}

// ─── Reference action ────────────────────────────────────────────────────────

fn reference_action_to_drizzle(action: &ReferenceAction) -> &'static str {
    match action {
        ReferenceAction::Cascade => "cascade",
        ReferenceAction::Restrict => "restrict",
        ReferenceAction::SetNull => "set null",
        ReferenceAction::SetDefault => "set default",
        // Includes NoAction and any unknown/future referential action.
        _ => "no action",
    }
}
