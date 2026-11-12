//! Painless Rust <-> JS bindings.
//!
//! ```ignore
//! ctx.bind("add", |a: i32, b: i32| a + b);
//! ctx.bind("console.log", |args: Rest<String>| println!("{}", args.0.join(" ")));
//! ctx.bind("fs.read", |p: String| std::fs::read_to_string(p)); // Err => JS exception
//! ctx.set("VERSION", "1.0");
//! ```
//!
//! Arguments implement [`FromJsValue`], return values implement [`IntoJs`].
//! A `Result<T, E: Display>` return throws a JS `Error` on `Err`.

use std::rc::Rc;

type Scope<'s, 'i> = v8::PinScope<'s, 'i>;
type Args<'s> = v8::FunctionCallbackArguments<'s>;

/// Type-erased native function stored in the isolate.
pub(crate) type NativeFn =
    Rc<dyn for<'s, 'i> Fn(&mut Scope<'s, 'i>, &Args<'s>) -> Result<v8::Local<'s, v8::Value>, String>>;

#[derive(Default)]
pub(crate) struct Registry(pub Vec<NativeFn>);

// ---------------------------------------------------------------- conversion

/// Convert a JS value into a Rust value.
pub trait FromJsValue: Sized {
    fn from_value(scope: &mut Scope<'_, '_>, value: v8::Local<'_, v8::Value>)
    -> Result<Self, String>;
}

/// Convert a Rust value into a JS value.
pub trait IntoJs {
    fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String>;
}

/// Collects every remaining argument. Must be the last parameter.
pub struct Rest<T>(pub Vec<T>);

/// Something that can be read from argument position `index`.
pub trait FromArg: Sized {
    fn from_arg<'s>(scope: &mut Scope<'s, '_>, args: &Args<'s>, index: i32) -> Result<Self, String>;
}

impl<T: FromJsValue> FromArg for T {
    fn from_arg<'s>(scope: &mut Scope<'s, '_>, args: &Args<'s>, index: i32) -> Result<Self, String> {
        T::from_value(scope, args.get(index)).map_err(|e| format!("argument {}: {e}", index + 1))
    }
}

impl<T: FromJsValue> FromArg for Rest<T> {
    fn from_arg<'s>(scope: &mut Scope<'s, '_>, args: &Args<'s>, index: i32) -> Result<Self, String> {
        (index..args.length())
            .map(|i| {
                T::from_value(scope, args.get(i)).map_err(|e| format!("argument {}: {e}", i + 1))
            })
            .collect::<Result<_, _>>()
            .map(Rest)
    }
}

impl FromJsValue for bool {
    fn from_value(scope: &mut Scope<'_, '_>, v: v8::Local<'_, v8::Value>) -> Result<Self, String> {
        if v.is_boolean() { Ok(v.boolean_value(scope)) } else { Err("expected a boolean".into()) }
    }
}

macro_rules! num_from_js {
    ($($t:ty),*) => {$(
        impl FromJsValue for $t {
            fn from_value(scope: &mut Scope<'_, '_>, v: v8::Local<'_, v8::Value>) -> Result<Self, String> {
                if !v.is_number() { return Err("expected a number".into()); }
                v.number_value(scope).map(|n| n as $t).ok_or_else(|| "expected a number".into())
            }
        }
        impl IntoJs for $t {
            fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String> {
                Ok(v8::Number::new(scope, self as f64).into())
            }
        }
    )*};
}
num_from_js!(i8, i16, i32, i64, isize, u8, u16, u32, u64, usize, f32, f64);

impl FromJsValue for String {
    fn from_value(scope: &mut Scope<'_, '_>, v: v8::Local<'_, v8::Value>) -> Result<Self, String> {
        if v.is_string() {
            Ok(v.to_rust_string_lossy(scope))
        } else {
            Err("expected a string".into())
        }
    }
}

impl<T: FromJsValue> FromJsValue for Option<T> {
    fn from_value(scope: &mut Scope<'_, '_>, v: v8::Local<'_, v8::Value>) -> Result<Self, String> {
        if v.is_null_or_undefined() { Ok(None) } else { T::from_value(scope, v).map(Some) }
    }
}

impl<T: FromJsValue> FromJsValue for Vec<T> {
    fn from_value(scope: &mut Scope<'_, '_>, v: v8::Local<'_, v8::Value>) -> Result<Self, String> {
        let arr = v8::Local::<v8::Array>::try_from(v).map_err(|_| "expected an array".to_string())?;
        (0..arr.length())
            .map(|i| {
                let item = arr.get_index(scope, i).ok_or("array access failed")?;
                T::from_value(scope, item)
            })
            .collect()
    }
}

impl IntoJs for () {
    fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String> {
        Ok(v8::undefined(scope).into())
    }
}

impl IntoJs for bool {
    fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String> {
        Ok(v8::Boolean::new(scope, self).into())
    }
}

impl IntoJs for String {
    fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String> {
        v8::String::new(scope, &self).map(Into::into).ok_or_else(|| "string too large".into())
    }
}

impl IntoJs for &str {
    fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String> {
        self.to_owned().into_js(scope)
    }
}

impl<T: IntoJs> IntoJs for Option<T> {
    fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String> {
        match self {
            Some(v) => v.into_js(scope),
            None => Ok(v8::null(scope).into()),
        }
    }
}

impl<T: IntoJs> IntoJs for Vec<T> {
    fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String> {
        let items = self.into_iter().map(|v| v.into_js(scope)).collect::<Result<Vec<_>, _>>()?;
        Ok(v8::Array::new_with_elements(scope, &items).into())
    }
}

impl<T: IntoJs, E: std::fmt::Display> IntoJs for Result<T, E> {
    fn into_js<'s>(self, scope: &mut Scope<'s, '_>) -> Result<v8::Local<'s, v8::Value>, String> {
        self.map_err(|e| e.to_string()).and_then(|v| v.into_js(scope))
    }
}

// ------------------------------------------------------------ function glue

/// Implemented for any `Fn(A, B, ..) -> R` whose arguments are [`FromArg`]
/// and whose return is [`IntoJs`]. `M` is just a marker to keep impls apart.
pub trait IntoNative<M>: 'static {
    fn into_native(self) -> NativeFn;
}

macro_rules! impl_into_native {
    ($($a:ident),*) => {
        impl<F, R, $($a),*> IntoNative<fn($($a),*) -> R> for F
        where
            F: Fn($($a),*) -> R + 'static,
            R: IntoJs,
            $($a: FromArg,)*
        {
            #[allow(non_snake_case, unused_mut, unused_variables, unused_assignments)]
            fn into_native(self) -> NativeFn {
                Rc::new(move |scope, args| {
                    let mut i = 0;
                    $(let $a = $a::from_arg(scope, args, i)?; i += 1;)*
                    (self)($($a),*).into_js(scope)
                })
            }
        }
    };
}
impl_into_native!();
impl_into_native!(A);
impl_into_native!(A, B);
impl_into_native!(A, B, C);
impl_into_native!(A, B, C, D);
impl_into_native!(A, B, C, D, E);
impl_into_native!(A, B, C, D, E, G);

/// The single C-callable entry point every bound function goes through.
fn trampoline<'s, 'i>(
    scope: &mut Scope<'s, 'i>,
    args: Args<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let native = args
        .data()
        .int32_value(scope)
        .and_then(|i| scope.get_slot::<Registry>()?.0.get(i as usize).cloned());

    let outcome = match native {
        Some(f) => f(scope, &args),
        None => Err("native function is no longer registered".to_string()),
    };

    match outcome {
        Ok(value) => rv.set(value),
        Err(msg) => throw(scope, &msg),
    }
}

fn throw(scope: &mut Scope<'_, '_>, msg: &str) {
    if let Some(msg) = v8::String::new(scope, msg) {
        let err = v8::Exception::error(scope, msg);
        scope.throw_exception(err);
    }
}

/// Register `f` and install it at `path` (e.g. `"console.log"`).
pub(crate) fn install<'s>(
    scope: &mut Scope<'s, '_>,
    path: &str,
    f: NativeFn,
) -> Result<(), String> {
    let registry = scope.get_slot_mut::<Registry>().ok_or("registry missing")?;
    registry.0.push(f);
    let index = (registry.0.len() - 1) as i32;

    let data = v8::Integer::new(scope, index);
    let func = v8::Function::builder(trampoline)
        .data(data.into())
        .build(scope)
        .ok_or("failed to create function")?;

    set_path(scope, path, func.into())
}

/// Set `path` (dotted) on the global object, creating parent objects as needed.
pub(crate) fn set_path<'s>(
    scope: &mut Scope<'s, '_>,
    path: &str,
    value: v8::Local<'s, v8::Value>,
) -> Result<(), String> {
    let (parents, name) = match path.rsplit_once('.') {
        Some((p, n)) => (p.split('.').collect::<Vec<_>>(), n),
        None => (Vec::new(), path),
    };

    let mut target = scope.get_current_context().global(scope);
    for seg in parents {
        let key: v8::Local<v8::Value> = v8::String::new(scope, seg).ok_or("bad path")?.into();
        let existing = target.get(scope, key).filter(|v| v.is_object());
        target = match existing {
            Some(v) => v8::Local::<v8::Object>::try_from(v).map_err(|_| "bad path")?,
            None => {
                let obj = v8::Object::new(scope);
                target.set(scope, key, obj.into());
                obj
            }
        };
    }

    let key: v8::Local<v8::Value> = v8::String::new(scope, name).ok_or("bad path")?.into();
    target.set(scope, key, value);
    Ok(())
}
