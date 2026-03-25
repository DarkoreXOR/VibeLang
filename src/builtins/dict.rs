use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{Param, TypeExpr};
use crate::builtins::{BuiltinError, BuiltinImpl};
use crate::error::Span;
use crate::value::Value;

fn dict_receiver_ty_ok(te: &TypeExpr) -> bool {
    match te {
        TypeExpr::EnumApp { name, args } => {
            name == "Dict" && args.len() == 2 && args.iter().all(|a| matches!(a, TypeExpr::TypeParam(_)))
        }
        _ => false,
    }
}

fn type_param(te: &TypeExpr) -> bool {
    matches!(te, TypeExpr::TypeParam(_) | TypeExpr::Named(_))
}

fn option_of_type_param(te: &TypeExpr) -> bool {
    match te {
        TypeExpr::EnumApp { name, args } => {
            name == "Option"
                && args.len() == 1
                && matches!(&args[0], TypeExpr::TypeParam(_) | TypeExpr::Named(_))
        }
        _ => false,
    }
}

fn option_none() -> Value {
    Value::Enum {
        enum_name: "Option".to_string(),
        variant: "None".to_string(),
        payloads: vec![],
    }
}

fn option_some(payload: Value) -> Value {
    Value::Enum {
        enum_name: "Option".to_string(),
        variant: "Some".to_string(),
        payloads: vec![payload],
    }
}

fn dict_entries_clone(dict: &Rc<RefCell<crate::value::StructInstance>>) -> Result<Vec<Value>, BuiltinError> {
    let inst = dict.borrow();
    let entries = inst
        .fields
        .get("entries")
        .ok_or_else(|| BuiltinError::new("Dict is missing `entries` field", None))?;

    match entries {
        Value::Array(arr) => Ok(arr.clone()),
        _ => Err(BuiltinError::new(
            "Dict `entries` field must be an array",
            None,
        )),
    }
}

pub struct DictContainsBuiltin;
pub struct DictGetBuiltin;
pub struct DictInsertBuiltin;
pub struct DictRemoveBuiltin;

impl BuiltinImpl for DictContainsBuiltin {
    fn name(&self) -> &'static str {
        "Dict<K, V>::contains"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 2 {
            return Err(BuiltinError::new(
                "internal `Dict::contains` must take `(self, key: K)`",
                Some(name_span),
            ));
        }
        if !dict_receiver_ty_ok(&params[0].ty) {
            return Err(BuiltinError::new(
                "internal `Dict::contains` must have `self: Dict<K, V>`",
                Some(name_span),
            ));
        }
        if !type_param(&params[1].ty) {
            return Err(BuiltinError::new(
                "internal `Dict::contains` must have `key: K`",
                Some(name_span),
            ));
        }
        let Some(rt) = return_type else {
            return Err(BuiltinError::new(
                "internal `Dict::contains` must return `Bool`",
                Some(name_span),
            ));
        };
        match rt {
            TypeExpr::Named(n) if n == "Bool" => Ok(()),
            _ => Err(BuiltinError::new(
                "internal `Dict::contains` must return `Bool`",
                Some(name_span),
            )),
        }
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        let mut it = args.into_iter();
        let dict = match it.next() {
            Some(Value::Struct(rc)) => rc,
            _ => {
                return Err(BuiltinError::new(
                    "`Dict::contains` expects a Dict value as `self`",
                    Some(span),
                ))
            }
        };
        let key = it.next().ok_or_else(|| {
            BuiltinError::new("`Dict::contains` expects `key`", Some(span))
        })?;

        let entries = dict_entries_clone(&dict)?;
        let mut found = false;
        for entry in entries.iter() {
            let Value::Tuple(parts) = entry else { continue };
            if parts.len() != 2 {
                continue;
            }
            if parts[0] == key {
                found = true;
                break;
            }
        }
        Ok(Value::Bool(found))
    }
}

impl BuiltinImpl for DictGetBuiltin {
    fn name(&self) -> &'static str {
        "Dict<K, V>::get"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 2 {
            return Err(BuiltinError::new(
                "internal `Dict::get` must take `(self, key: K)`",
                Some(name_span),
            ));
        }
        if !dict_receiver_ty_ok(&params[0].ty) {
            return Err(BuiltinError::new(
                "internal `Dict::get` must have `self: Dict<K, V>`",
                Some(name_span),
            ));
        }
        if !type_param(&params[1].ty) {
            return Err(BuiltinError::new(
                "internal `Dict::get` must have `key: K`",
                Some(name_span),
            ));
        }
        let Some(rt) = return_type else {
            return Err(BuiltinError::new(
                "internal `Dict::get` must return `Option<V>`",
                Some(name_span),
            ));
        };
        if !option_of_type_param(rt) {
            return Err(BuiltinError::new(
                "internal `Dict::get` must return `Option<V>`",
                Some(name_span),
            ));
        }
        Ok(())
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        let mut it = args.into_iter();
        let dict = match it.next() {
            Some(Value::Struct(rc)) => rc,
            _ => {
                return Err(BuiltinError::new(
                    "`Dict::get` expects a Dict value as `self`",
                    Some(span),
                ))
            }
        };
        let key = it.next().ok_or_else(|| {
            BuiltinError::new("`Dict::get` expects `key`", Some(span))
        })?;

        let entries = dict_entries_clone(&dict)?;
        for entry in entries.iter() {
            let Value::Tuple(parts) = entry else { continue };
            if parts.len() != 2 {
                continue;
            }
            if parts[0] == key {
                return Ok(option_some(parts[1].clone()));
            }
        }
        Ok(option_none())
    }
}

impl BuiltinImpl for DictInsertBuiltin {
    fn name(&self) -> &'static str {
        "Dict<K, V>::insert"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 3 {
            return Err(BuiltinError::new(
                "internal `Dict::insert` must take `(self, key: K, value: V)`",
                Some(name_span),
            ));
        }
        if !dict_receiver_ty_ok(&params[0].ty) {
            return Err(BuiltinError::new(
                "internal `Dict::insert` must have `self: Dict<K, V>`",
                Some(name_span),
            ));
        }
        if !type_param(&params[1].ty) || !type_param(&params[2].ty) {
            return Err(BuiltinError::new(
                "internal `Dict::insert` must have `key: K` and `value: V`",
                Some(name_span),
            ));
        }
        let Some(rt) = return_type else {
            return Err(BuiltinError::new(
                "internal `Dict::insert` must return `Option<V>`",
                Some(name_span),
            ));
        };
        if !option_of_type_param(rt) {
            return Err(BuiltinError::new(
                "internal `Dict::insert` must return `Option<V>`",
                Some(name_span),
            ));
        }
        Ok(())
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        let mut it = args.into_iter();
        let dict = match it.next() {
            Some(Value::Struct(rc)) => rc,
            _ => {
                return Err(BuiltinError::new(
                    "`Dict::insert` expects a Dict value as `self`",
                    Some(span),
                ))
            }
        };
        let key = it.next().ok_or_else(|| {
            BuiltinError::new("`Dict::insert` expects `key`", Some(span))
        })?;
        let value = it.next().ok_or_else(|| {
            BuiltinError::new("`Dict::insert` expects `value`", Some(span))
        })?;

        let mut inst = dict.borrow_mut();
        let entries = inst
            .fields
            .get_mut("entries")
            .ok_or_else(|| BuiltinError::new("Dict is missing `entries` field", Some(span)))?;

        let Value::Array(arr) = entries else {
            return Err(BuiltinError::new("Dict `entries` field must be an array", Some(span)));
        };

        for entry in arr.iter_mut() {
            let Value::Tuple(parts) = entry else { continue };
            if parts.len() != 2 {
                continue;
            }
            if parts[0] == key {
                let old = parts[1].clone();
                parts[1] = value;
                return Ok(option_some(old));
            }
        }

        arr.push(Value::Tuple(vec![key, value]));
        Ok(option_none())
    }
}

impl BuiltinImpl for DictRemoveBuiltin {
    fn name(&self) -> &'static str {
        "Dict<K, V>::remove"
    }

    fn validate_decl(
        &self,
        params: &[Param],
        return_type: &Option<TypeExpr>,
        name_span: Span,
    ) -> Result<(), BuiltinError> {
        if params.len() != 2 {
            return Err(BuiltinError::new(
                "internal `Dict::remove` must take `(self, key: K)`",
                Some(name_span),
            ));
        }
        if !dict_receiver_ty_ok(&params[0].ty) {
            return Err(BuiltinError::new(
                "internal `Dict::remove` must have `self: Dict<K, V>`",
                Some(name_span),
            ));
        }
        if !type_param(&params[1].ty) {
            return Err(BuiltinError::new(
                "internal `Dict::remove` must have `key: K`",
                Some(name_span),
            ));
        }
        let Some(rt) = return_type else {
            return Err(BuiltinError::new(
                "internal `Dict::remove` must return `Option<V>`",
                Some(name_span),
            ));
        };
        if !option_of_type_param(rt) {
            return Err(BuiltinError::new(
                "internal `Dict::remove` must return `Option<V>`",
                Some(name_span),
            ));
        }
        Ok(())
    }

    fn eval(&self, args: Vec<Value>, span: Span) -> Result<Value, BuiltinError> {
        let mut it = args.into_iter();
        let dict = match it.next() {
            Some(Value::Struct(rc)) => rc,
            _ => {
                return Err(BuiltinError::new(
                    "`Dict::remove` expects a Dict value as `self`",
                    Some(span),
                ))
            }
        };
        let key = it.next().ok_or_else(|| {
            BuiltinError::new("`Dict::remove` expects `key`", Some(span))
        })?;

        let mut inst = dict.borrow_mut();
        let entries = inst
            .fields
            .get_mut("entries")
            .ok_or_else(|| BuiltinError::new("Dict is missing `entries` field", Some(span)))?;

        let Value::Array(arr) = entries else {
            return Err(BuiltinError::new("Dict `entries` field must be an array", Some(span)));
        };

        let mut i = 0usize;
        while i < arr.len() {
            let remove = match &arr[i] {
                Value::Tuple(parts) if parts.len() == 2 => parts[0] == key,
                _ => false,
            };
            if remove {
                let old_entry = arr.remove(i);
                if let Value::Tuple(parts) = old_entry {
                    let old_val = parts[1].clone();
                    return Ok(option_some(old_val));
                }
                break;
            }
            i += 1;
        }

        Ok(option_none())
    }
}

