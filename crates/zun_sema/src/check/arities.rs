/// `Function.length` of the `Date.prototype` methods (spec arities).
pub(in crate::check) fn date_function_length(name: &str) -> Option<f64> {
    Some(match name {
        "setUTCFullYear" | "setFullYear" => 3.0,
        "setUTCMonth" | "setMonth" | "setUTCSeconds" | "setSeconds" => 2.0,
        "setUTCHours" | "setHours" => 4.0,
        "setUTCMinutes" | "setMinutes" => 3.0,
        "setUTCDate" | "setDate" | "setUTCMilliseconds" | "setMilliseconds" | "setTime" => 1.0,
        "getTime" | "valueOf" | "toISOString" | "toString" | "toUTCString" | "getUTCFullYear"
        | "getUTCMonth" | "getUTCDate" | "getUTCDay" | "getUTCHours" | "getUTCMinutes"
        | "getUTCSeconds" | "getUTCMilliseconds" | "getFullYear" | "getMonth" | "getDate"
        | "getDay" | "getHours" | "getMinutes" | "getSeconds" | "getMilliseconds"
        | "getTimezoneOffset" | "toDateString" | "toTimeString" => 0.0,
        "toJSON" => 1.0,
        _ => return None,
    })
}

/// `Function.length` of the `Math` methods (spec arities).
pub(in crate::check) fn math_function_length(name: &str) -> Option<f64> {
    Some(match name {
        "max" | "min" | "hypot" | "pow" | "atan2" | "imul" => 2.0,
        "abs" | "acos" | "acosh" | "asin" | "asinh" | "atan" | "atanh" | "cbrt" | "ceil"
        | "clz32" | "cos" | "cosh" | "exp" | "expm1" | "floor" | "fround" | "log" | "log10"
        | "log1p" | "log2" | "round" | "sign" | "sin" | "sinh" | "sqrt" | "tan" | "tanh"
        | "trunc" => 1.0,
        "random" => 0.0,
        _ => return None,
    })
}

/// Spec `length` of well-known built-in functions addressed by a dotted path
/// such as `["Map", "groupBy"]` or `["Promise", "prototype", "then"]`.
pub(in crate::check) fn builtin_function_length(path: &[&str]) -> Option<f64> {
    Some(match path {
        ["Map", "groupBy"] | ["Object", "groupBy"] => 2.0,
        ["Promise", "all" | "race" | "allSettled" | "any" | "resolve" | "reject"] => 1.0,
        ["Promise", "withResolvers"] => 0.0,
        ["Promise", "prototype", "then"] => 2.0,
        ["Promise", "prototype", "catch" | "finally"] => 1.0,
        ["Map" | "Set" | "WeakMap" | "WeakSet", "prototype", "get" | "has" | "delete" | "add"] => 1.0,
        ["Map" | "WeakMap", "prototype", "set"] => 2.0,
        ["Map" | "Set", "prototype", "clear"] => 0.0,
        ["Map" | "Set", "prototype", "forEach"] => 1.0,
        ["Object", "keys" | "values" | "entries" | "freeze" | "isFrozen" | "getPrototypeOf" | "fromEntries"] => 1.0,
        ["Object", "assign" | "defineProperty" ] => if path[1] == "assign" { 2.0 } else { 3.0 },
        ["Object", "getOwnPropertyDescriptor" | "hasOwn" | "setPrototypeOf"] => 2.0,
        ["String", "prototype", "repeat" | "at" | "charAt" | "charCodeAt" | "codePointAt" | "normalize" | "indexOf" | "lastIndexOf" | "includes" | "startsWith" | "endsWith" | "concat" | "localeCompare"] => {
            if path[2] == "normalize" { 0.0 } else { 1.0 }
        }
        ["String", "prototype", "padStart" | "padEnd" | "split" | "slice" | "substring" | "replace" | "replaceAll"] => 2.0,
        ["String", "prototype", "trim" | "trimStart" | "trimEnd" | "toUpperCase" | "toLowerCase" | "toString" | "valueOf"] => 0.0,
        ["Array", "prototype", "push" | "unshift" | "concat" | "at" | "every" | "fill" | "filter" | "find" | "findIndex" | "findLast" | "findLastIndex" | "flatMap" | "forEach" | "includes" | "indexOf" | "join" | "lastIndexOf" | "map" | "reduce" | "reduceRight" | "some" | "sort" | "toSorted"] => 1.0,
        ["Array", "prototype", "copyWithin" | "slice" | "splice" | "toSpliced" | "with"] => 2.0,
        ["Array", "prototype", "pop" | "shift" | "entries" | "keys" | "values" | "flat" | "reverse" | "toLocaleString" | "toReversed" | "toString"] => 0.0,
        ["Array", "isArray" | "of"] => 1.0,
        ["Array", "from"] => 1.0,
        ["Number", "isInteger" | "isNaN" | "isFinite" | "isSafeInteger" | "parseFloat"] => 1.0,
        ["Number", "parseInt"] => 2.0,
        _ => return None,
    })
}
