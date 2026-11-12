use std::cell::RefCell;
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

    /// Evaluate `source` and convert the result, e.g. `ctx.eval_as::<i32>("1 + 2")`.
    pub fn eval_as<T: FromJsValue>(&mut self, source: &str) -> Result<T, EvalError> {
        self.with_scope(|scope| {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let try_catch = &mut try_catch.init();
            let code = v8::String::new(try_catch, source).ok_or(EvalError::InvalidSource)?;
            let value = v8::Script::compile(try_catch, code, None)
                .and_then(|script| script.run(try_catch));
            match value {
                Some(value) => T::from_value(try_catch, value).map_err(EvalError::Binding),
                None => Err(EvalError::Exception(
                    try_catch
                        .exception()
                        .map(|e| e.to_rust_string_lossy(try_catch))
                        .unwrap_or_else(|| "unknown exception".into()),
                )),
            }
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

    /// Compile and run `source`, returning the result converted to a string.
    pub fn eval(&mut self, source: &str) -> Result<String, EvalError> {
        self.with_scope(|scope| {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let try_catch = &mut try_catch.init();

            let code = v8::String::new(try_catch, source).ok_or(EvalError::InvalidSource)?;

            let value =
                v8::Script::compile(try_catch, code, None).and_then(|script| script.run(try_catch));

            match value {
                Some(value) => Ok(value
                    .to_string(try_catch)
                    .map(|s| s.to_rust_string_lossy(try_catch))
                    .unwrap_or_default()),
                None => {
                    let msg = try_catch
                        .exception()
                        .map(|e| e.to_rust_string_lossy(try_catch))
                        .unwrap_or_else(|| "unknown exception".into());
                    Err(EvalError::Exception(msg))
                }
            }
        })
    }
}
