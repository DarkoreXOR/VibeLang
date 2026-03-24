//! Structural type keys for extension dispatch in bytecode (no full `Ty` environment).

use crate::ast::TypeExpr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TyKey {
    Ident(String),
    Unit,
    Tuple(Vec<TyKey>),
    Array(Box<TyKey>),
    EnumApp { name: String, args: Vec<TyKey> },
}

/// Build a structural key from a type expression (e.g. first `self` parameter of an extension).
pub fn type_expr_to_ty_key(te: &TypeExpr) -> Option<TyKey> {
    match te {
        TypeExpr::Named(n) => Some(TyKey::Ident(n.clone())),
        TypeExpr::Unit => Some(TyKey::Unit),
        TypeExpr::Infer => None,
        TypeExpr::Tuple(parts) => {
            let mut t = Vec::new();
            for p in parts {
                t.push(type_expr_to_ty_key(p)?);
            }
            Some(TyKey::Tuple(t))
        }
        TypeExpr::Array(elem) => Some(TyKey::Array(Box::new(type_expr_to_ty_key(elem)?))),
        TypeExpr::EnumApp { name, args } => {
            let mut a = Vec::new();
            for p in args {
                a.push(type_expr_to_ty_key(p)?);
            }
            Some(TyKey::EnumApp {
                name: name.clone(),
                args: a,
            })
        }
        TypeExpr::TypeParam(n) => Some(TyKey::Ident(n.clone())),
        TypeExpr::Function { .. } => None,
    }
}

/// Format a key compatible with `ty_to_receiver_key` / `receiver_key_from_type_expr` (comma + space).
pub fn format_ty_key(k: &TyKey) -> String {
    match k {
        TyKey::Ident(s) => s.clone(),
        TyKey::Unit => "()".to_string(),
        TyKey::Tuple(parts) => {
            let mut out = String::from("(");
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format_ty_key(p));
            }
            if parts.len() == 1 {
                out.push(',');
            }
            out.push(')');
            out
        }
        TyKey::Array(inner) => format!("[{}]", format_ty_key(inner)),
        TyKey::EnumApp { name, args } => {
            let inner = args
                .iter()
                .map(|a| format_ty_key(a))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}<{}>", name, inner)
        }
    }
}

/// True if `inferred` matches `template`, treating identifiers listed in `type_params` as wildcards.
pub fn match_type_keys(template: &TyKey, inferred: &TyKey, type_params: &[String]) -> bool {
    match (template, inferred) {
        (TyKey::Ident(tn), _) if type_params.contains(tn) => true,
        (TyKey::Ident(a), TyKey::Ident(b)) => a == b,
        (TyKey::Unit, TyKey::Unit) => true,
        (TyKey::Array(te), TyKey::Array(ie)) => match_type_keys(te, ie, type_params),
        (TyKey::Tuple(t1), TyKey::Tuple(t2)) if t1.len() == t2.len() => t1
            .iter()
            .zip(t2.iter())
            .all(|(t, i)| match_type_keys(t, i, type_params)),
        (
            TyKey::EnumApp {
                name: n1,
                args: a1,
            },
            TyKey::EnumApp {
                name: n2,
                args: a2,
            },
        ) => {
            n1 == n2
                && a1.len() == a2.len()
                && a1
                    .iter()
                    .zip(a2.iter())
                    .all(|(t, i)| match_type_keys(t, i, type_params))
        }
        _ => false,
    }
}
