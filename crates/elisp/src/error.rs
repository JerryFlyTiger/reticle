use crate::value::Value;

/// Non-local exits: error signals and catch/throw.
pub enum Flow {
    Signal { error_symbol: Value, data: Value },
    Throw { tag: Value, value: Value },
}

pub type EvalResult = Result<Value, Flow>;
