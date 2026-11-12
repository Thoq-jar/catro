use super::{Rest, V8Context};
use crate::scripting::error::EvalError;

pub fn init_v8_context() -> Result<V8Context, EvalError> {
    let mut context = V8Context::new()?;
    context.bind("console.log", |args: Rest<String>| {
        println!("{}", args.0.join(" "))
    })?;
    context.set("VERSION", "1.0")?;

    Ok(context)
}
