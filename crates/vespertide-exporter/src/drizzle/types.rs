use vespertide_core::schema::column::{ColumnType, ComplexColumnType, EnumValues, SimpleColumnType};

use super::enums::to_camel_case;

/// Return the bare Drizzle `pg-core` function name for this column type (used
/// for collecting the import symbol set). `pgEnum`-derived names are returned
/// as their camelCase const name and excluded from the `pg-core` import list by
/// the caller.
pub(super) fn col_type_symbol(ty: &ColumnType, is_auto_increment: bool) -> String {
    match ty {
        ColumnType::Simple(s) => match s {
            SimpleColumnType::SmallInt => "smallint".to_string(),
            SimpleColumnType::Integer => {
                if is_auto_increment {
                    "serial".to_string()
                } else {
                    "integer".to_string()
                }
            }
            SimpleColumnType::BigInt => {
                if is_auto_increment {
                    "bigserial".to_string()
                } else {
                    "bigint".to_string()
                }
            }
            SimpleColumnType::Real => "real".to_string(),
            SimpleColumnType::DoublePrecision => "doublePrecision".to_string(),
            SimpleColumnType::Boolean => "boolean".to_string(),
            SimpleColumnType::Date => "date".to_string(),
            SimpleColumnType::Time => "time".to_string(),
            SimpleColumnType::Timestamp | SimpleColumnType::Timestamptz => "timestamp".to_string(),
            SimpleColumnType::Uuid => "uuid".to_string(),
            SimpleColumnType::Json => "jsonb".to_string(),
            // `Text` plus unsupported/future pg types fall back to `text`.
            _ => "text".to_string(),
        },
        ColumnType::Complex(c) => match c {
            ComplexColumnType::Varchar { .. } => "varchar".to_string(),
            ComplexColumnType::Char { .. } => "char".to_string(),
            ComplexColumnType::Numeric { .. } => "numeric".to_string(),
            ComplexColumnType::Enum { name, values } => match values {
                // Integer enums are plain integer columns — no pgEnum declaration.
                EnumValues::Integer(_) => "integer".to_string(),
                EnumValues::String(_) => to_camel_case(name),
            },
            // Custom and unknown/future complex types fall back to `text`.
            _ => "text".to_string(),
        },
    }
}

/// Render the full Drizzle `pg-core` column constructor call for a column,
/// e.g. `varchar("email", { length: 255 })`.
pub(super) fn render_col_type_call(
    ty: &ColumnType,
    col_db: &str,
    is_auto_increment: bool,
) -> String {
    match ty {
        ColumnType::Simple(s) => match s {
            SimpleColumnType::SmallInt => format!("smallint(\"{col_db}\")"),
            SimpleColumnType::Integer => {
                if is_auto_increment {
                    format!("serial(\"{col_db}\")")
                } else {
                    format!("integer(\"{col_db}\")")
                }
            }
            SimpleColumnType::BigInt => {
                if is_auto_increment {
                    format!("bigserial(\"{col_db}\", {{ mode: \"number\" }})")
                } else {
                    format!("bigint(\"{col_db}\", {{ mode: \"number\" }})")
                }
            }
            SimpleColumnType::Real => format!("real(\"{col_db}\")"),
            SimpleColumnType::DoublePrecision => format!("doublePrecision(\"{col_db}\")"),
            SimpleColumnType::Boolean => format!("boolean(\"{col_db}\")"),
            SimpleColumnType::Date => format!("date(\"{col_db}\")"),
            SimpleColumnType::Time => format!("time(\"{col_db}\")"),
            SimpleColumnType::Timestamp => format!("timestamp(\"{col_db}\")"),
            SimpleColumnType::Timestamptz => {
                format!("timestamp(\"{col_db}\", {{ withTimezone: true }})")
            }
            SimpleColumnType::Uuid => format!("uuid(\"{col_db}\")"),
            SimpleColumnType::Json => format!("jsonb(\"{col_db}\")"),
            SimpleColumnType::Interval => format!("text(\"{col_db}\") /* interval */"),
            SimpleColumnType::Bytea => format!("text(\"{col_db}\") /* bytea */"),
            SimpleColumnType::Inet => format!("text(\"{col_db}\") /* inet */"),
            SimpleColumnType::Cidr => format!("text(\"{col_db}\") /* cidr */"),
            SimpleColumnType::Macaddr => format!("text(\"{col_db}\") /* macaddr */"),
            SimpleColumnType::Xml => format!("text(\"{col_db}\") /* xml */"),
            // Unknown/future simple types fall back to a plain text column.
            _ => format!("text(\"{col_db}\")"),
        },
        ColumnType::Complex(c) => match c {
            ComplexColumnType::Varchar { length } => {
                format!("varchar(\"{col_db}\", {{ length: {length} }})")
            }
            ComplexColumnType::Char { length } => {
                format!("char(\"{col_db}\", {{ length: {length} }})")
            }
            ComplexColumnType::Numeric { precision, scale } => {
                format!("numeric(\"{col_db}\", {{ precision: {precision}, scale: {scale} }})")
            }
            ComplexColumnType::Custom { custom_type } => {
                format!("text(\"{col_db}\") /* {custom_type} */")
            }
            ComplexColumnType::Enum { name, values } => match values {
                // Integer enums map to plain integer columns — no pgEnum type.
                EnumValues::Integer(_) => format!("integer(\"{col_db}\")"),
                EnumValues::String(_) => {
                    let enum_camel = to_camel_case(name);
                    format!("{enum_camel}(\"{col_db}\")")
                }
            },
            // Unknown/future complex types fall back to a plain text column.
            _ => format!("text(\"{col_db}\")"),
        },
    }
}
