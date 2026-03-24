use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::AstNode;
use crate::lexer::Lexer;
use crate::parser::Parser;

#[derive(Debug, Clone)]
pub struct ModuleLoadError {
    pub path: Option<PathBuf>,
    pub message: String,
}

impl ModuleLoadError {
    fn with_path(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            path: Some(path.into()),
            message: message.into(),
        }
    }

    fn plain(message: impl Into<String>) -> Self {
        Self {
            path: None,
            message: message.into(),
        }
    }
}

#[derive(Clone)]
struct ImportDecl {
    names: Vec<String>,
    module_path: String,
}

#[derive(Clone)]
struct ModuleData {
    items: Vec<AstNode>,
    imports: Vec<ImportDecl>,
    exports: HashSet<String>,
}

pub fn load_linked_program(entry_file: &str) -> Result<AstNode, ModuleLoadError> {
    let workspace = std::env::current_dir()
        .map_err(|e| ModuleLoadError::plain(format!("failed to read current directory: {e}")))?;
    let entry_path = canonicalize_path(Path::new(entry_file), &workspace)?;

    let mut loaded: HashMap<PathBuf, ModuleData> = HashMap::new();
    let mut visiting: HashSet<PathBuf> = HashSet::new();
    let mut order: Vec<PathBuf> = Vec::new();
    load_module_recursive(
        &workspace,
        &entry_path,
        &mut loaded,
        &mut visiting,
        &mut order,
    )?;

    // Explicit-import visibility:
    // - entry module keeps all of its declarations
    // - dependency modules expose only names explicitly imported from them
    // - non-exported declarations are still included (module internals)
    let required_exports = collect_required_exports(&workspace, &entry_path, &loaded)?;

    let mut merged = Vec::new();
    for path in order {
        let Some(module) = loaded.get(&path) else {
            continue;
        };
        let required_for_module = required_exports.get(&path);
        for item in &module.items {
            if matches!(item, AstNode::Import { .. }) {
                continue;
            }
            if path != entry_path {
                if let Some((name, is_exported)) = item_named_export(item) {
                    if is_exported {
                        let is_required = required_for_module
                            .is_some_and(|needed| needed.contains(name));
                        if !is_required {
                            continue;
                        }
                    }
                }
            }
            merged.push(strip_export(item));
        }
    }

    Ok(AstNode::Program(merged))
}

fn item_named_export(item: &AstNode) -> Option<(&str, bool)> {
    match item {
        AstNode::InternalFunction {
            name, is_exported, ..
        }
        | AstNode::Function {
            name, is_exported, ..
        }
        | AstNode::StructDef {
            name, is_exported, ..
        }
        | AstNode::EnumDef {
            name, is_exported, ..
        } => Some((name.as_str(), *is_exported)),
        _ => None,
    }
}

fn collect_required_exports(
    workspace: &Path,
    entry_path: &Path,
    loaded: &HashMap<PathBuf, ModuleData>,
) -> Result<HashMap<PathBuf, HashSet<String>>, ModuleLoadError> {
    fn visit(
        module_path: &Path,
        workspace: &Path,
        loaded: &HashMap<PathBuf, ModuleData>,
        required: &mut HashMap<PathBuf, HashSet<String>>,
        seen: &mut HashSet<PathBuf>,
    ) -> Result<(), ModuleLoadError> {
        if !seen.insert(module_path.to_path_buf()) {
            return Ok(());
        }
        let Some(module) = loaded.get(module_path) else {
            return Err(ModuleLoadError::with_path(
                module_path,
                "dependency not loaded",
            ));
        };
        for imp in &module.imports {
            let dep_path = resolve_import_path(workspace, module_path, &imp.module_path)
                .ok_or_else(|| {
                    ModuleLoadError::with_path(
                        module_path,
                        format!("cannot resolve module path `{}`", imp.module_path),
                    )
                })?;
            let needed = required.entry(dep_path.clone()).or_default();
            for n in &imp.names {
                needed.insert(n.clone());
            }
            visit(&dep_path, workspace, loaded, required, seen)?;
        }
        Ok(())
    }

    let mut required: HashMap<PathBuf, HashSet<String>> = HashMap::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    visit(entry_path, workspace, loaded, &mut required, &mut seen)?;
    Ok(required)
}

fn load_module_recursive(
    workspace: &Path,
    module_path: &Path,
    loaded: &mut HashMap<PathBuf, ModuleData>,
    visiting: &mut HashSet<PathBuf>,
    order: &mut Vec<PathBuf>,
) -> Result<(), ModuleLoadError> {
    if loaded.contains_key(module_path) {
        return Ok(());
    }
    if !visiting.insert(module_path.to_path_buf()) {
        return Err(ModuleLoadError::with_path(
            module_path,
            "cyclic module import detected",
        ));
    }

    let data = parse_module_file(module_path)?;
    for imp in &data.imports {
        let dep_path = resolve_import_path(workspace, module_path, &imp.module_path)
            .ok_or_else(|| {
                ModuleLoadError::with_path(
                    module_path,
                    format!("cannot resolve module path `{}`", imp.module_path),
                )
            })?;
        load_module_recursive(workspace, &dep_path, loaded, visiting, order)?;
        let dep = loaded
            .get(&dep_path)
            .ok_or_else(|| ModuleLoadError::with_path(&dep_path, "dependency not loaded"))?;
        for name in &imp.names {
            if !dep.exports.contains(name) {
                return Err(ModuleLoadError::with_path(
                    module_path,
                    format!("module `{}` does not export `{name}`", imp.module_path),
                ));
            }
        }
    }

    visiting.remove(module_path);
    loaded.insert(module_path.to_path_buf(), data);
    order.push(module_path.to_path_buf());
    Ok(())
}

fn parse_module_file(path: &Path) -> Result<ModuleData, ModuleLoadError> {
    let source = fs::read_to_string(path)
        .map_err(|e| ModuleLoadError::with_path(path, format!("failed reading module: {e}")))?;
    let mut lexer = Lexer::new(&source);
    let tokens = lexer
        .tokenize()
        .map_err(|e| ModuleLoadError::with_path(path, e.to_string()))?;
    let mut parser = Parser::new(tokens);
    let ast = parser
        .parse()
        .map_err(|e| ModuleLoadError::with_path(path, e.to_string()))?;
    let AstNode::Program(items) = ast else {
        return Err(ModuleLoadError::with_path(path, "parser did not return a program"));
    };

    let mut imports = Vec::new();
    let mut exports = HashSet::new();
    for item in &items {
        match item {
            AstNode::Import {
                names, module_path, ..
            } => imports.push(ImportDecl {
                names: names.iter().map(|(n, _)| n.clone()).collect(),
                module_path: module_path.clone(),
            }),
            AstNode::InternalFunction {
                name, is_exported, ..
            }
            | AstNode::Function {
                name, is_exported, ..
            }
            | AstNode::StructDef {
                name, is_exported, ..
            }
            | AstNode::EnumDef {
                name, is_exported, ..
            } => {
                if *is_exported {
                    exports.insert(name.clone());
                }
            }
            _ => {}
        }
    }

    Ok(ModuleData {
        items,
        imports,
        exports,
    })
}

fn resolve_import_path(workspace: &Path, current_file: &Path, spec: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if spec.starts_with("./") || spec.starts_with("../") {
        let base = current_file.parent().unwrap_or_else(|| Path::new("."));
        candidates.push(base.join(spec));
    } else {
        candidates.push(workspace.join(spec));
    }

    for c in candidates {
        for cand in expand_module_candidate_paths(&c) {
            if cand.exists() {
                if let Ok(real) = canonicalize_path(&cand, workspace) {
                    return Some(real);
                }
            }
        }
    }
    None
}

fn expand_module_candidate_paths(base: &Path) -> Vec<PathBuf> {
    if base.extension().is_some() {
        vec![base.to_path_buf()]
    } else {
        vec![
            base.with_extension("vc"),
            base.join("mod.vc"),
            base.join("core.vc"),
        ]
    }
}

fn canonicalize_path(path: &Path, workspace: &Path) -> Result<PathBuf, ModuleLoadError> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    joined
        .canonicalize()
        .map_err(|e| ModuleLoadError::with_path(path, format!("invalid module path: {e}")))
}

fn strip_export(item: &AstNode) -> AstNode {
    match item {
        AstNode::InternalFunction {
            name,
            type_params,
            params,
            return_type,
            name_span,
            is_async,
            ..
        } => AstNode::InternalFunction {
            name: name.clone(),
            type_params: type_params.clone(),
            params: params.clone(),
            return_type: return_type.clone(),
            name_span: *name_span,
            is_exported: false,
            is_async: *is_async,
        },
        AstNode::Function {
            name,
            extension_receiver,
            type_params,
            params,
            return_type,
            body,
            name_span,
            closing_span,
            is_async,
            ..
        } => AstNode::Function {
            name: name.clone(),
            extension_receiver: extension_receiver.clone(),
            type_params: type_params.clone(),
            params: params.clone(),
            return_type: return_type.clone(),
            body: body.clone(),
            name_span: *name_span,
            closing_span: *closing_span,
            is_exported: false,
            is_async: *is_async,
        },
        AstNode::StructDef {
            name,
            type_params,
            fields,
            is_unit,
            is_internal,
            name_span,
            span,
            ..
        } => AstNode::StructDef {
            name: name.clone(),
            type_params: type_params.clone(),
            fields: fields.clone(),
            is_unit: *is_unit,
            is_internal: *is_internal,
            name_span: *name_span,
            span: *span,
            is_exported: false,
        },
        AstNode::EnumDef {
            name,
            type_params,
            variants,
            name_span,
            span,
            ..
        } => AstNode::EnumDef {
            name: name.clone(),
            type_params: type_params.clone(),
            variants: variants.clone(),
            name_span: *name_span,
            span: *span,
            is_exported: false,
        },
        _ => item.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode_gen::compile_program;
    use crate::semantic::check_program;
    use crate::vm::run_program;

    #[test]
    fn load_example18_modules_and_run() {
        let restore_dir = std::env::current_dir().expect("cwd");
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::set_current_dir(&manifest_dir).expect("chdir");
        let ast = load_linked_program("examples/example18.vc").expect("load");
        std::env::set_current_dir(restore_dir).expect("restore cwd");
        let errs = check_program(&ast);
        assert!(errs.is_empty(), "{:?}", errs);
        let bytecode = compile_program(&ast).expect("compile");
        run_program(&bytecode).expect("run");
    }

    #[test]
    fn load_example20_extensions_and_run() {
        let restore_dir = std::env::current_dir().expect("cwd");
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::set_current_dir(&manifest_dir).expect("chdir");
        let ast = load_linked_program("examples/example20.vc").expect("load");
        std::env::set_current_dir(restore_dir).expect("restore cwd");
        let errs = check_program(&ast);
        assert!(errs.is_empty(), "{:?}", errs);
        let bytecode = compile_program(&ast).expect("compile");
        run_program(&bytecode).expect("run");
    }

    #[test]
    fn load_example21_generic_array_extensions_and_run() {
        let restore_dir = std::env::current_dir().expect("cwd");
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::set_current_dir(&manifest_dir).expect("chdir");
        let ast = load_linked_program("examples/example21.vc").expect("load");
        std::env::set_current_dir(restore_dir).expect("restore cwd");
        let errs = check_program(&ast);
        assert!(errs.is_empty(), "{:?}", errs);
        let bytecode = compile_program(&ast).expect("compile");
        run_program(&bytecode).expect("run");
    }

    #[test]
    fn import_non_exported_symbol_fails() {
        let base = std::env::temp_dir().join("vibelang_module_test_non_export");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("examples")).expect("mkdir");
        fs::write(base.join("m.vc"), "func hidden() {}\nexport func pubf() {}").expect("write m");
        fs::write(
            base.join("examples").join("main.vc"),
            "import { hidden } from \"m\";\nfunc main() { hidden(); }",
        )
        .expect("write main");

        let restore_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::set_current_dir(&base).expect("chdir");
        let res = load_linked_program("examples/main.vc");
        std::env::set_current_dir(restore_dir).expect("restore cwd");
        let _ = fs::remove_dir_all(&base);

        let err = res.expect_err("expected link error");
        assert!(err.message.contains("does not export `hidden`"), "{:?}", err);
    }

    #[test]
    fn unimported_exported_symbol_is_not_visible() {
        let base = std::env::temp_dir().join("vibelang_module_test_explicit_imports");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("std")).expect("mkdir");
        fs::create_dir_all(base.join("examples")).expect("mkdir");
        fs::write(
            base.join("std").join("core.vc"),
            "export enum Result<T, E> { Ok(T), Err(E) }\nexport internal func print_gen<T>(t: T);",
        )
        .expect("write core");
        fs::write(
            base.join("examples").join("main.vc"),
            "import { print_gen } from \"std/core\";\nfunc main() { print_gen(Result<_, Int>::Ok(1)); }",
        )
        .expect("write main");

        let restore_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        std::env::set_current_dir(&base).expect("chdir");
        let ast = load_linked_program("examples/main.vc").expect("load");
        let errs = check_program(&ast);
        std::env::set_current_dir(restore_dir).expect("restore cwd");
        let _ = fs::remove_dir_all(&base);

        assert!(
            errs.iter().any(|e| e.message.contains("unknown enum `Result`")),
            "{:?}",
            errs
        );
    }
}
