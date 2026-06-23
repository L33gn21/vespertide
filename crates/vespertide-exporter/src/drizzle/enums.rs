use vespertide_core::schema::column::EnumValues;

/// Render a `pgEnum` declaration:
/// `export const status = pgEnum("status", ["a", "b"]);`
pub(super) fn render_enum_decl(name: &str, const_name: &str, values: &EnumValues) -> String {
    let variants: Vec<String> = match values {
        EnumValues::String(vals) => vals.iter().map(|v| format!("\"{v}\"")).collect(),
        EnumValues::Integer(vals) => vals.iter().map(|v| format!("\"{}\"", v.name)).collect(),
    };
    format!(
        "export const {const_name} = pgEnum(\"{name}\", [{}]);",
        variants.join(", ")
    )
}

/// Infer the relation field name from a FK column by stripping a trailing `_id`.
pub(super) fn infer_relation_field_name(fk_col: &str) -> String {
    fk_col.strip_suffix("_id").unwrap_or(fk_col).to_string()
}

/// snake_case → camelCase (e.g. `user_profiles` → `userProfiles`).
pub(super) fn to_camel_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;
    for (i, ch) in s.char_indices() {
        if ch == '_' {
            if i > 0 {
                capitalize_next = true;
            }
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// snake_case → PascalCase (e.g. `user_profiles` → `UserProfiles`).
pub(super) fn to_pascal_case(s: &str) -> String {
    s.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect()
}
