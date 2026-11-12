use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Once;

use crate::scripting::PRELUDE;
use crate::scripting::bind::{self, FromJsValue, IntoJs, IntoNative, Registry};
use crate::scripting::error::EvalError;

/// A V8 context that can be handed around freely.
pub struct V8Context {
    // The global handle must drop before the isolate.
    context: v8::Global<v8::Context>,
    isolate: v8::OwnedIsolate,
}

/// Module resolution state, stored in an isolate slot.
struct ModuleLoader {
    /// Directory that relative imports from the entry module resolve against.
    base: PathBuf,
    /// In-memory modules, importable by exact name.
    virtuals: HashMap<String, String>,
    /// Compiled dependencies, keyed by canonical path or virtual name.
    cache: HashMap<String, v8::Global<v8::Module>>,
    /// Module identity hash -> directory its own imports resolve against.
    dirs: HashMap<i32, PathBuf>,
}

impl Default for ModuleLoader {
    fn default() -> Self {
        Self {
            base: std::env::current_dir().unwrap_or_default(),
            virtuals: HashMap::new(),
            cache: HashMap::new(),
            dirs: HashMap::new(),
        }
    }
}

impl Drop for V8Context {
    fn drop(&mut self) {
        // Globals must be released while the isolate is still alive.
        if let Some(loader) = self.isolate.get_slot_mut::<ModuleLoader>() {
            loader.cache.clear();
        }
    }
}

pub type SharedV8Context = Rc<RefCell<V8Context>>;

fn init_platform() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

impl V8Context {
    /// Create a context and run the embedded prelude in it.
    pub fn new() -> Result<Self, EvalError> {
        init_platform();

        let mut isolate = v8::Isolate::new(Default::default());
        isolate.set_slot(Registry::default());
        isolate.set_slot(ModuleLoader::default());

        let context = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let local = v8::Context::new(scope, Default::default());
            v8::Global::new(scope, local)
        };

        let mut this = Self { context, isolate };
        this.run_with_global(PRELUDE)?; // load prelude
        Ok(this)
    }

    /// Directory that `import "./x.js"` in the entry module resolves against.
    pub fn set_base_dir(&mut self, dir: impl Into<PathBuf>) {
        if let Some(loader) = self.isolate.get_slot_mut::<ModuleLoader>() {
            loader.base = dir.into();
        }
    }

    /// Read and run the file at `path` as an ES module, using its directory as
    /// the root for imports. Returns the default export as a string.
    pub fn run_file(&mut self, path: impl AsRef<Path>) -> Result<String, EvalError> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path)
            .map_err(|e| EvalError::Binding(format!("cannot read {}: {e}", path.display())))?;
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
        self.set_base_dir(dir);
        self.eval(&source)
    }

    /// Make an in-memory module importable as `import ... from "<name>"`.
    pub fn register_module(&mut self, name: &str, source: &str) {
        if let Some(loader) = self.isolate.get_slot_mut::<ModuleLoader>() {
            loader.virtuals.insert(name.into(), source.into());
        }
    }

    pub fn into_shared(self) -> SharedV8Context {
        Rc::new(RefCell::new(self))
    }

    /// Enter the context and run `f` with a ready-to-use scope.
    pub fn with_scope<R>(
        &mut self,
        f: impl FnOnce(&mut v8::ContextScope<'_, '_, v8::HandleScope<'_>>) -> R,
    ) -> R {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut self.isolate));
        let scope = &mut scope.init();
        let local = v8::Local::new(scope, &self.context);
        let scope = &mut v8::ContextScope::new(scope, local);
        f(scope)
    }

    /// Bind a Rust closure to a JS global, e.g. `ctx.bind("math.add", |a: i32, b: i32| a + b)`.
    /// Dotted paths create the intermediate objects.
    pub fn bind<M>(&mut self, path: &str, f: impl IntoNative<M>) -> Result<(), EvalError> {
        let native = f.into_native();
        self.with_scope(|scope| bind::install(scope, path, native)).map_err(EvalError::Binding)
    }

    /// Expose a plain value as a JS global, e.g. `ctx.set("VERSION", "1.0")`.
    pub fn set(&mut self, path: &str, value: impl IntoJs) -> Result<(), EvalError> {
        self.with_scope(|scope| {
            let value = value.into_js(scope)?;
            bind::set_path(scope, path, value)
        })
        .map_err(EvalError::Binding)
    }

    /// Read a global by dotted path and convert it, e.g.
    /// `ctx.get::<String>("config.name")`. Errors if the path is missing.
    pub fn get<T: FromJsValue>(&mut self, path: &str) -> Result<T, EvalError> {
        self.with_scope(|scope| {
            let mut value: v8::Local<v8::Value> = scope.get_current_context().global(scope).into();
            for seg in path.split('.') {
                let obj = v8::Local::<v8::Object>::try_from(value)
                    .map_err(|_| EvalError::Binding(format!("'{path}' is not defined")))?;
                let key = v8::String::new(scope, seg).ok_or(EvalError::InvalidSource)?;
                value = obj
                    .get(scope, key.into())
                    .filter(|v| !v.is_undefined())
                    .ok_or_else(|| EvalError::Binding(format!("'{path}' is not defined")))?;
            }
            T::from_value(scope, value).map_err(EvalError::Binding)
        })
    }

    /// Evaluate `source` as an ES module and convert its default export,
    /// e.g. `ctx.eval_as::<i32>("export default 1 + 2")`.
    pub fn eval_as<T: FromJsValue>(&mut self, source: &str) -> Result<T, EvalError> {
        self.with_scope(|scope| {
            let value = run_module(scope, source)?;
            T::from_value(scope, value).map_err(EvalError::Binding)
        })
    }

    /// Evaluate `source` as a function expression and call it with the
    /// context's global object as its only argument (`globalScope`).
    pub fn run_with_global(&mut self, source: &str) -> Result<(), EvalError> {
        self.with_scope(|scope| {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let try_catch = &mut try_catch.init();

            let code = v8::String::new(try_catch, source).ok_or(EvalError::InvalidSource)?;
            let global = try_catch.get_current_context().global(try_catch);

            let result = v8::Script::compile(try_catch, code, None)
                .and_then(|script| script.run(try_catch))
                .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
                .and_then(|func| func.call(try_catch, global.into(), &[global.into()]));

            match result {
                Some(_) => Ok(()),
                None => {
                    let msg = try_catch
                        .exception()
                        .map(|e| e.to_rust_string_lossy(try_catch))
                        .unwrap_or_else(|| "source did not evaluate to a function".into());
                    Err(EvalError::Exception(msg))
                }
            }
        })
    }

    /// Evaluate `source` as an ES module (the default), returning its default
    /// export converted to a string (`"undefined"` if there is none).
    pub fn eval(&mut self, source: &str) -> Result<String, EvalError> {
        self.with_scope(|scope| {
            let value = run_module(scope, source)?;
            Ok(value.to_rust_string_lossy(scope))
        })
    }

    /// Evaluate `source` as a CommonJS module, returning `module.exports`
    /// converted to a string. `exports`, `module` and `require` are in scope;
    /// `require` uses a global `require` function if one is defined.
    pub fn eval_common(&mut self, source: &str) -> Result<String, EvalError> {
        self.with_scope(|scope| {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let try_catch = &mut try_catch.init();

            let wrapped = format!(
                "(function (exports, module, require) {{\n{source}\n}})"
            );
            let code = v8::String::new(try_catch, &wrapped).ok_or(EvalError::InvalidSource)?;
            let global = try_catch.get_current_context().global(try_catch);

            let result = (|| {
                let func = v8::Script::compile(try_catch, code, None)?.run(try_catch)?;
                let func = v8::Local::<v8::Function>::try_from(func).ok()?;

                let module = v8::Object::new(try_catch);
                let exports = v8::Object::new(try_catch);
                let key = v8::String::new(try_catch, "exports")?;
                module.set(try_catch, key.into(), exports.into())?;

                let req_key = v8::String::new(try_catch, "require")?;
                let require = match global.get(try_catch, req_key.into()) {
                    Some(v) if v.is_function() => v,
                    _ => {
                        let stub = v8::Script::compile(
                            try_catch,
                            v8::String::new(try_catch, "(function (id) { throw new Error('require is not available: ' + id); })")?,
                            None,
                        )?
                        .run(try_catch)?;
                        stub
                    }
                };

                func.call(
                    try_catch,
                    exports.into(),
                    &[exports.into(), module.into(), require],
                )?;
                module.get(try_catch, key.into())
            })();

            match result {
                Some(v) => Ok(v
                    .to_string(try_catch)
                    .map(|s| s.to_rust_string_lossy(try_catch))
                    .unwrap_or_default()),
                None => Err(EvalError::Exception(exception_text(try_catch))),
            }
        })
    }
}

fn exception_text(try_catch: &mut v8::PinnedRef<'_, v8::TryCatch<v8::HandleScope>>) -> String {
    try_catch
        .exception()
        .map(|e| e.to_rust_string_lossy(try_catch))
        .unwrap_or_else(|| "unknown exception".into())
}

/// Compile `source` as an ES module named `name`.
fn compile_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: &str,
    name: &str,
) -> Option<v8::Local<'s, v8::Module>> {
    let code = v8::String::new(scope, source)?;
    let name = v8::String::new(scope, name)?;
    let origin = v8::ScriptOrigin::new(
        scope, name.into(), 0, 0, false, 0, None, false, false, true, None,
    );
    let mut src = v8::script_compiler::Source::new(code, Some(&origin));
    v8::script_compiler::compile_module(scope, &mut src)
}

fn throw_error(scope: &mut v8::PinScope<'_, '_>, msg: &str) {
    if let Some(msg) = v8::String::new(scope, msg) {
        let err = v8::Exception::error(scope, msg);
        scope.throw_exception(err);
    }
}

/// Find the file an import specifier refers to.
fn locate(dir: &Path, specifier: &str) -> Option<PathBuf> {
    let p = dir.join(specifier);
    let candidates = [
        p.clone(),
        p.with_extension("js"),
        p.with_extension("mjs"),
        p.join("index.js"),
    ];
    candidates.into_iter().find(|c| c.is_file()).and_then(|c| c.canonicalize().ok())
}

/// V8 calls this for every `import`; resolves, compiles and caches the target.
fn resolve_import<'a>(
    context: v8::Local<'a, v8::Context>,
    specifier: v8::Local<'a, v8::String>,
    _attributes: v8::Local<'a, v8::FixedArray>,
    referrer: v8::Local<'a, v8::Module>,
) -> Option<v8::Local<'a, v8::Module>> {
    v8::callback_scope!(unsafe scope, context);
    let spec = specifier.to_rust_string_lossy(scope);
    let referrer_hash = referrer.get_identity_hash().get();

    // Work out what to load without holding the slot borrow across V8 calls.
    enum Target {
        Cached(v8::Global<v8::Module>),
        Load { key: String, source: String, dir: PathBuf },
    }
    let target = {
        let loader = scope.get_slot::<ModuleLoader>()?;
        if let Some(source) = loader.virtuals.get(&spec) {
            match loader.cache.get(&spec) {
                Some(m) => Target::Cached(m.clone()),
                None => Target::Load {
                    key: spec.clone(),
                    source: source.clone(),
                    dir: loader.base.clone(),
                },
            }
        } else {
            // `./x` and `../x` resolve against the importing file; anything
            // else (`config.js`, `lib/x`) resolves against the root directory.
            let dir = if spec.starts_with("./") || spec.starts_with("../") {
                loader.dirs.get(&referrer_hash).unwrap_or(&loader.base)
            } else {
                &loader.base
            };
            match locate(dir, &spec) {
                Some(path) => {
                    let key = path.to_string_lossy().into_owned();
                    match loader.cache.get(&key) {
                        Some(m) => Target::Cached(m.clone()),
                        None => match std::fs::read_to_string(&path) {
                            Ok(source) => Target::Load {
                                key,
                                source,
                                dir: path.parent().map(Path::to_path_buf).unwrap_or_default(),
                            },
                            Err(e) => {
                                let msg = format!("cannot read module '{spec}': {e}");
                                throw_error(scope, &msg);
                                return None;
                            }
                        },
                    }
                }
                None => {
                    throw_error(scope, &format!("cannot find module '{spec}'"));
                    return None;
                }
            }
        }
    };

    match target {
        Target::Cached(global) => Some(v8::Local::new(scope, &global)),
        Target::Load { key, source, dir } => {
            let module = compile_module(scope, &source, &key)?;
            let global = v8::Global::new(scope, module);
            let loader = scope.get_slot_mut::<ModuleLoader>()?;
            loader.dirs.insert(module.get_identity_hash().get(), dir);
            loader.cache.insert(key, global);
            Some(module)
        }
    }
}

/// Compile, instantiate and evaluate `source` as an ES module; returns its
/// default export (or `undefined`). Relative imports resolve against the base
/// directory; `register_module` names resolve to in-memory sources.
fn run_module<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    source: &str,
) -> Result<v8::Local<'s, v8::Value>, EvalError> {
    // Start each run fresh so edited files are picked up.
    let base = {
        let loader = scope
            .get_slot_mut::<ModuleLoader>()
            .ok_or_else(|| EvalError::Binding("module loader missing".into()))?;
        loader.cache.clear();
        loader.dirs.clear();
        loader.base.clone()
    };

    let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
    let try_catch = &mut try_catch.init();

    let entry = base.join("module.js");
    let Some(module) = compile_module(try_catch, source, &entry.to_string_lossy()) else {
        return Err(EvalError::Exception(exception_text(try_catch)));
    };
    if let Some(loader) = try_catch.get_slot_mut::<ModuleLoader>() {
        loader.dirs.insert(module.get_identity_hash().get(), base);
    }
    if module.instantiate_module(try_catch, resolve_import) != Some(true) {
        return Err(EvalError::Exception(exception_text(try_catch)));
    }
    let Some(promise) = module.evaluate(try_catch) else {
        return Err(EvalError::Exception(exception_text(try_catch)));
    };
    try_catch.perform_microtask_checkpoint();

    if let Ok(promise) = v8::Local::<v8::Promise>::try_from(promise) {
        match promise.state() {
            v8::PromiseState::Rejected => {
                let err = promise.result(try_catch);
                return Err(EvalError::Exception(err.to_rust_string_lossy(try_catch)));
            }
            v8::PromiseState::Pending => {
                return Err(EvalError::Exception("module did not finish evaluating".into()));
            }
            v8::PromiseState::Fulfilled => {}
        }
    }

    let ns = module.get_module_namespace();
    let default_key = v8::String::new(try_catch, "default").ok_or(EvalError::InvalidSource)?;
    let value = v8::Local::<v8::Object>::try_from(ns)
        .ok()
        .and_then(|ns| ns.get(try_catch, default_key.into()))
        .unwrap_or_else(|| v8::undefined(try_catch).into());
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_common() {
        let mut ctx = V8Context::new().unwrap();
        assert_eq!(ctx.eval("export default 1 + 2").unwrap(), "3");
        assert_eq!(ctx.eval_as::<i32>("const a = 4; export default a * 2").unwrap(), 8);
        assert!(ctx.eval("import x from 'y'; export default x").is_err());

        ctx.register_module("lib", "export const n = 20; export default 1;");
        assert_eq!(ctx.eval("import d, { n } from 'lib'; export default d + n").unwrap(), "21");

        let dir = std::env::temp_dir().join(format!("catro_mod_{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.js"), "import { b } from './sub/b.js'; export const a = b + 1;").unwrap();
        std::fs::write(dir.join("sub/b.js"), "export const b = 10;").unwrap();
        ctx.set_base_dir(&dir);
        assert_eq!(ctx.eval("import { a } from './a'; export default a").unwrap(), "11");
        std::fs::write(dir.join("main.js"), "import { a } from 'a.js'; export default a * 2").unwrap();
        assert_eq!(ctx.run_file(dir.join("main.js")).unwrap(), "22");
        let err = ctx.eval("import './nope.js'").unwrap_err().to_string();
        assert!(err.contains("cannot find module"), "{err}");
        ctx.eval("globalThis.cfg = { name: 'x', size: { w: 3 } }; export default 0").unwrap();
        assert_eq!(ctx.get::<String>("cfg.name").unwrap(), "x");
        assert_eq!(ctx.get::<i32>("cfg.size.w").unwrap(), 3);
        assert!(ctx.get::<i32>("cfg.nope.w").is_err());
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(ctx.eval_common("module.exports = 5; ").unwrap(), "5");
        assert_eq!(ctx.eval_common("exports.a = 1;").unwrap(), "[object Object]");
        assert!(ctx.eval_common("require('x')").is_err());
    }
}
