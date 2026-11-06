//! Dynamic `String.prototype` methods with full ECMAScript argument coercion.
//!
//! The statically-typed string natives take exact `string`/integer arguments.
//! JS code passes optional, `undefined`, fractional, negative, or non-number
//! arguments; those calls are routed here, where every argument goes through
//! `ToIntegerOrInfinity` / `ToString` exactly as the specification describes.
//! Indices are UTF-16 code units.

use super::collections::{is_truthy, js_string_to_number, unbox_arg};
use super::value::Value;
use super::{Interp, RunError};

fn units(text: &str) -> Vec<u16> {
    zun_core::wtf16::encode(text).collect()
}

fn from_units(units: &[u16]) -> String {
    zun_core::wtf16::from_units(units)
}

/// ToIntegerOrInfinity on an already-numeric value (`-0` folds to `+0`).
fn integer_or_infinity(number: f64) -> f64 {
    if number.is_nan() {
        0.0
    } else if number.is_infinite() {
        number
    } else {
        number.trunc() + 0.0
    }
}

/// Clamp an integer-or-infinity into `[0, len]`.
fn clamp_index(number: f64, len: usize) -> usize {
    if number <= 0.0 {
        0
    } else if number >= len as f64 {
        len
    } else {
        number as usize
    }
}

/// Resolve a relative index (negative counts from the end) into `[0, len]`.
fn relative_index(number: f64, len: usize) -> usize {
    if number < 0.0 {
        let resolved = len as f64 + number;
        if resolved <= 0.0 {
            0
        } else {
            resolved as usize
        }
    } else {
        clamp_index(number, len)
    }
}

fn find_from(haystack: &[u16], needle: &[u16], from: usize) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    (from..=last).find(|&start| haystack[start..start + needle.len()] == *needle)
}

fn number(value: f64) -> Value {
    Value::Float(value, zun_il::NumKind::F64)
}

impl<'a> Interp<'a> {
    /// ECMAScript ToNumber over any value (objects run OrdinaryToPrimitive).
    pub(super) fn dyn_to_number(&mut self, value: &Value) -> Result<f64, RunError> {
        Ok(match unbox_arg(value)? {
            Value::Int(n, _) => n as f64,
            Value::Float(n, _) => n,
            Value::Bool(flag) => f64::from(u8::from(flag)),
            Value::Null => 0.0,
            Value::Void => f64::NAN,
            Value::Str(text) => js_string_to_number(&text),
            Value::Object(obj) => match self.ordinary_to_primitive(obj, "number")? {
                Some(Value::Str(text)) => js_string_to_number(&text),
                Some(Value::Int(n, _)) => n as f64,
                Some(Value::Float(n, _)) => n,
                Some(Value::Bool(flag)) => f64::from(u8::from(flag)),
                Some(Value::Null) => 0.0,
                _ => f64::NAN,
            },
            _ => f64::NAN,
        })
    }

    /// ECMAScript ToString over any value (objects run OrdinaryToPrimitive).
    pub(super) fn dyn_to_string(&mut self, value: &Value) -> Result<String, RunError> {
        if let Ok(Value::Any(any)) = unbox_arg(value) {
            if any.as_symbol().is_some() {
                return Err(RunError::Native(
                    "TypeError: Cannot convert a Symbol value to a string".into(),
                ));
            }
        }
        Ok(match unbox_arg(value)? {
            Value::Object(obj) => {
                if let Some(text) = zun_gc::with_current_heap(|heap| {
                    heap.regex_source(obj)
                        .map(|source| format!("/{source}/{}", heap.regex_flags(obj).unwrap_or("")))
                }) {
                    return Ok(text);
                }
                // An array stringifies as its comma-joined elements.
                if let Some(len) = zun_gc::with_current_heap(|heap| heap.array_len(obj)) {
                    let mut parts: Vec<String> = Vec::with_capacity(len);
                    for index in 0..len {
                        let element = zun_gc::with_current_heap(|heap| heap.array_get(obj, index));
                        parts.push(match element {
                            Some(any) => match unbox_arg(&Value::Any(any))? {
                                Value::Void | Value::Null => String::new(),
                                other => self.dyn_to_string(&other)?,
                            },
                            None => String::new(),
                        });
                    }
                    return Ok(parts.join(","));
                }
                match self.ordinary_to_primitive(obj, "string")? {
                    Some(primitive) => primitive.display(),
                    None => "[object Object]".to_string(),
                }
            }
            other => other.display(),
        })
    }

    fn dyn_integer(&mut self, value: &Value) -> Result<f64, RunError> {
        Ok(integer_or_infinity(self.dyn_to_number(value)?))
    }

    /// ECMAScript IsRegExp: a truthy `Symbol.match` property, else a real regexp.
    fn is_regexp(&mut self, value: &Value) -> Result<bool, RunError> {
        let Ok(Value::Object(obj)) = unbox_arg(value) else { return Ok(false) };
        // A `Symbol.match` accessor is read exactly once.
        let getter = zun_gc::with_current_heap(|heap| {
            heap.dynamic_symbol_getter(obj, zun_core::WellKnownSymbol::Match.id())
        });
        if let Some(getter) = getter {
            let args = self.with_this_arg(getter, obj, Vec::new());
            let result = self.invoke_function_value(getter, args)?;
            return Ok(zun_gc::with_current_heap(|heap| is_truthy(&result, heap)));
        }
        Ok(zun_gc::with_current_heap(|heap| {
            match heap.dynamic_get_symbol(obj, zun_core::WellKnownSymbol::Match.id()) {
                Some(any) => is_truthy(&Value::Any(any), heap),
                None => heap.regex_source(obj).is_some(),
            }
        }))
    }

    /// Whether an argument is `undefined` (an absent argument arrives as one).
    fn is_undefined(value: &Value) -> bool {
        matches!(unbox_arg(value), Ok(Value::Void))
    }

    /// Run `recv.name(...args)` with JS argument coercion. Unknown methods
    /// return `None` so the caller can fall back to its other dispatch.
    pub(super) fn string_method_dyn(
        &mut self,
        recv: &str,
        name: &str,
        args: &[Value],
    ) -> Result<Option<Value>, RunError> {
        let absent = Value::Void;
        let a0 = args.first().unwrap_or(&absent).clone();
        let a1 = args.get(1).unwrap_or(&absent).clone();
        let s = units(recv);
        let len = s.len();
        let text = |u: &[u16]| Value::Str(from_units(u).into());
        Ok(Some(match name {
            "substring" => {
                let start = clamp_index(self.dyn_integer(&a0)?, len);
                let end = if Self::is_undefined(&a1) {
                    len
                } else {
                    clamp_index(self.dyn_integer(&a1)?, len)
                };
                text(&s[start.min(end)..start.max(end)])
            }
            "slice" => {
                let start = relative_index(self.dyn_integer(&a0)?, len);
                let end = if Self::is_undefined(&a1) {
                    len
                } else {
                    relative_index(self.dyn_integer(&a1)?, len)
                };
                if start >= end {
                    Value::Str("".into())
                } else {
                    text(&s[start..end])
                }
            }
            "substr" => {
                let start = relative_index(self.dyn_integer(&a0)?, len);
                let count = if Self::is_undefined(&a1) {
                    len as f64
                } else {
                    self.dyn_integer(&a1)?
                };
                let end = (start as f64 + count.max(0.0)).min(len as f64) as usize;
                if start >= end {
                    Value::Str("".into())
                } else {
                    text(&s[start..end])
                }
            }
            "indexOf" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let from = clamp_index(self.dyn_integer(&a1)?, len);
                match find_from(&s, &needle, from) {
                    Some(index) => number(index as f64),
                    None => number(-1.0),
                }
            }
            "lastIndexOf" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let position = self.dyn_to_number(&a1)?;
                let from = if position.is_nan() {
                    len
                } else {
                    clamp_index(integer_or_infinity(position), len)
                };
                if needle.len() > len {
                    number(-1.0)
                } else {
                    let start = from.min(len - needle.len());
                    match (0..=start)
                        .rev()
                        .find(|&at| s[at..at + needle.len()] == *needle)
                    {
                        Some(index) => number(index as f64),
                        None => number(-1.0),
                    }
                }
            }
            "includes" | "startsWith" | "endsWith" if self.is_regexp(&a0)? => {
                return Err(RunError::Native(format!(
                    "TypeError: First argument to String.prototype.{name} must not be a regular expression"
                )));
            }
            "repeat" => {
                if let Ok(Value::Any(any)) = unbox_arg(&a0) {
                    if any.as_symbol().is_some() {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert a Symbol value to a number".into(),
                        ));
                    }
                }
                if let Ok(Value::Object(obj)) = unbox_arg(&a0) {
                    if zun_gc::with_current_heap(|heap| heap.bigint_value(obj).is_some()) {
                        return Err(RunError::Native(
                            "TypeError: Cannot convert a BigInt value to a number".into(),
                        ));
                    }
                }
                // Arrays count as their joined text (`[2]` is 2).
                let count_number = match unbox_arg(&a0)? {
                    Value::Object(obj) if crate::collections::array_join_text(obj).is_some() => {
                        js_string_to_number(&crate::collections::array_join_text(obj).unwrap_or_default())
                    }
                    _ => self.dyn_to_number(&a0)?,
                };
                let count = integer_or_infinity(count_number);
                if count < 0.0 || count.is_infinite() {
                    return Err(RunError::Native(format!(
                        "RangeError: Invalid count value: {}",
                        self.dyn_to_string(&a0)?
                    )));
                }
                if !recv.is_empty() && count > 0.0 && (recv.len() as f64) * count > 536_870_888.0 {
                    return Err(RunError::Native("RangeError: Invalid string length".into()));
                }
                if recv.is_empty() {
                    Value::Str("".into())
                } else {
                    Value::Str(zun_core::wtf16::normalize(recv.repeat(count as usize)).into())
                }
            }
            "includes" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let from = clamp_index(self.dyn_integer(&a1)?, len);
                Value::Bool(find_from(&s, &needle, from).is_some())
            }
            "startsWith" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let from = clamp_index(self.dyn_integer(&a1)?, len);
                Value::Bool(from + needle.len() <= len && s[from..from + needle.len()] == *needle)
            }
            "endsWith" => {
                let needle = units(&self.dyn_to_string(&a0)?);
                let end = if Self::is_undefined(&a1) {
                    len
                } else {
                    clamp_index(self.dyn_integer(&a1)?, len)
                };
                Value::Bool(needle.len() <= end && s[end - needle.len()..end] == *needle)
            }
            "padStart" | "padEnd" => {
                let target = self.dyn_integer(&a0)?;
                let fill = if Self::is_undefined(&a1) {
                    units(" ")
                } else {
                    units(&self.dyn_to_string(&a1)?)
                };
                if target <= len as f64 || fill.is_empty() {
                    Value::Str(recv.into())
                } else {
                    let missing = target as usize - len;
                    let padding: Vec<u16> = fill.iter().cycle().take(missing).copied().collect();
                    let mut out = Vec::with_capacity(target as usize);
                    if name == "padStart" {
                        out.extend_from_slice(&padding);
                        out.extend_from_slice(&s);
                    } else {
                        out.extend_from_slice(&s);
                        out.extend_from_slice(&padding);
                    }
                    text(&out)
                }
            }
            "charAt" => {
                let index = self.dyn_integer(&a0)?;
                if index >= 0.0 && index < len as f64 {
                    text(&s[index as usize..index as usize + 1])
                } else {
                    Value::Str("".into())
                }
            }
            "charCodeAt" => {
                let index = self.dyn_integer(&a0)?;
                if index >= 0.0 && index < len as f64 {
                    number(f64::from(s[index as usize]))
                } else {
                    number(f64::NAN)
                }
            }
            "codePointAt" => {
                let index = self.dyn_integer(&a0)?;
                if index >= 0.0 && index < len as f64 {
                    let i = index as usize;
                    let first = u32::from(s[i]);
                    if (0xD800..0xDC00).contains(&first) && i + 1 < len {
                        let second = u32::from(s[i + 1]);
                        if (0xDC00..0xE000).contains(&second) {
                            return Ok(Some(number(f64::from(
                                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00),
                            ))));
                        }
                    }
                    number(f64::from(first))
                } else {
                    Value::Void
                }
            }
            "at" => {
                let index = self.dyn_integer(&a0)?;
                let resolved = if index < 0.0 { len as f64 + index } else { index };
                if resolved >= 0.0 && resolved < len as f64 {
                    text(&s[resolved as usize..resolved as usize + 1])
                } else {
                    Value::Void
                }
            }
            "split" if !self.is_regexp(&a0)? => {
                // `split(separator?, limit?)` with a string (or absent) separator.
                let limit = if Self::is_undefined(&a1) {
                    usize::MAX
                } else {
                    let n = self.dyn_to_number(&a1)?;
                    if n.is_nan() || n <= 0.0 { 0 } else if n >= 4294967295.0 { usize::MAX } else { n as usize }
                };
                let pieces: Vec<String> = if Self::is_undefined(&a0) {
                    vec![recv.to_owned()]
                } else {
                    let separator = units(&self.dyn_to_string(&a0)?);
                    if separator.is_empty() {
                        s.iter().map(|unit| from_units(&[*unit])).collect()
                    } else {
                        let mut parts = Vec::new();
                        let mut from = 0;
                        while let Some(at) = find_from(&s, &separator, from) {
                            parts.push(from_units(&s[from..at]));
                            from = at + separator.len();
                        }
                        parts.push(from_units(&s[from..]));
                        parts
                    }
                };
                let pieces: Vec<String> = pieces.into_iter().take(limit).collect();
                let array = zun_gc::with_current_heap_mut(|heap| {
                    let array = heap.alloc_array(pieces.len());
                    for (index, piece) in pieces.iter().enumerate() {
                        let id = heap.intern_string(piece.as_str().into());
                        heap.array_set(array, index, zun_core::Any::string(id));
                    }
                    array
                });
                Value::Object(array)
            }
            "normalize" => {
                let form = if Self::is_undefined(&a0) { "NFC".to_string() } else { self.dyn_to_string(&a0)? };
                let normalized = zun_natives::string_normalize_form(recv, &form).ok_or_else(|| {
                    RunError::Native("RangeError: The normalization form should be one of NFC, NFD, NFKC, NFKD.".into())
                })?;
                Value::Str(normalized.into())
            }
            "toString" | "valueOf" => Value::Str(recv.into()),
            "toUpperCase" => Value::Str(recv.to_uppercase().into()),
            "toLowerCase" => Value::Str(recv.to_lowercase().into()),
            _ => return Ok(None),
        }))
    }
}

/// Outcome of a `GetMethod(obj, @@symbol)` walk.
enum SymbolLookup {
    Getter(zun_core::FunctionHandle),
    Function(zun_core::FunctionHandle),
    Value(zun_core::Any),
    Missing,
}

impl<'a> Interp<'a> {
    /// ECMAScript `GetMethod(obj, symbol)` over the object's prototype chain:
    /// `Ok(None)` for `undefined`/`null`, a `TypeError` for any other
    /// non-callable value.
    pub(super) fn symbol_method(
        &mut self,
        start: zun_gc::Obj,
        symbol: zun_core::SymbolId,
    ) -> Result<Option<zun_core::FunctionHandle>, RunError> {
        let mut current = start;
        for _ in 0..64 {
            let found = zun_gc::with_current_heap(|heap| {
                if let Some(getter) = heap.dynamic_symbol_getter(current, symbol) {
                    SymbolLookup::Getter(getter)
                } else if let Some(function) = heap.dynamic_get_symbol_function(current, symbol) {
                    SymbolLookup::Function(function)
                } else if let Some(any) = heap.dynamic_get_symbol(current, symbol) {
                    SymbolLookup::Value(any)
                } else {
                    SymbolLookup::Missing
                }
            });
            match found {
                SymbolLookup::Function(function) => return Ok(Some(function)),
                SymbolLookup::Getter(getter) => {
                    let args = self.with_this_arg(getter, start, Vec::new());
                    let value = self.invoke_function_value(getter, args)?;
                    return self.callable_or_nullish(&value);
                }
                SymbolLookup::Value(any) => return self.callable_or_nullish(&Value::Any(any)),
                SymbolLookup::Missing => {
                    let next = zun_gc::with_current_heap(|heap| heap.dynamic_prototype(current))
                        .and_then(|proto| proto.as_object());
                    match next {
                        Some(addr) => current = unsafe { zun_gc::Obj::from_addr(addr as *mut u64) },
                        None => return Ok(None),
                    }
                }
            }
        }
        Ok(None)
    }

    fn callable_or_nullish(&self, value: &Value) -> Result<Option<zun_core::FunctionHandle>, RunError> {
        match unbox_arg(value)? {
            Value::Void | Value::Null => Ok(None),
            other => match self.as_callable(&other) {
                Some(handle) => Ok(Some(handle)),
                None => Err(RunError::Native("TypeError: value is not a function".into())),
            },
        }
    }

    /// The callable a value stands for (a function, a function cell or a
    /// `functionBox` record), if any.
    pub(super) fn as_callable(&self, value: &Value) -> Option<zun_core::FunctionHandle> {
        match value {
            Value::Function(handle) => Some(*handle),
            Value::Object(object) => zun_gc::with_current_heap(|heap| {
                heap.function_of_cell(*object).or_else(|| heap.dynamic_get_function(*object, "__boxedFn"))
            }),
            _ => None,
        }
    }

    /// `str.match/replace/search/split/matchAll/replaceAll(arg, extra?)` with a
    /// dynamic `arg`: call `arg[Symbol.<kind>](str, extra?)` when present,
    /// otherwise the String.prototype algorithm (kind 0 match, 1 replace,
    /// 2 search, 3 split, 4 matchAll, 5 replaceAll).
    pub(super) fn string_symbol_dispatch(&mut self, args: &[Value]) -> Result<Value, RunError> {
        let receiver = match args.first().ok_or(RunError::BadToken)? {
            Value::Str(text) => text.clone(),
            _ => return Err(RunError::BadToken),
        };
        let arg = unbox_arg(args.get(1).ok_or(RunError::BadToken)?)?;
        let extra = unbox_arg(args.get(2).ok_or(RunError::BadToken)?)?;
        let kind = match args.get(3) {
            Some(Value::Int(kind, _)) => *kind,
            _ => return Err(RunError::BadToken),
        };
        let symbol = match kind {
            0 => zun_core::WellKnownSymbol::Match,
            1 | 5 => zun_core::WellKnownSymbol::Replace,
            2 => zun_core::WellKnownSymbol::Search,
            3 => zun_core::WellKnownSymbol::Split,
            4 | 6 => zun_core::WellKnownSymbol::MatchAll,
            _ => return Err(RunError::BadToken),
        };
        let regex_obj = match &arg {
            Value::Object(obj) if zun_gc::with_current_heap(|heap| heap.regex_source(*obj).is_some()) => Some(*obj),
            _ => None,
        };
        // matchAll / replaceAll refuse a regexp-like argument without `g`.
        if (kind == 4 || kind == 5) && (regex_obj.is_some() || self.is_regexp(&arg)?) {
            let flags = match (&arg, regex_obj) {
                (_, Some(obj)) => zun_gc::with_current_heap(|heap| heap.regex_flags(obj).unwrap_or("").to_owned()),
                (Value::Object(obj), None) => match zun_gc::with_current_heap(|heap| heap.dynamic_get(*obj, "flags")) {
                    Some(any) => self.dyn_to_string(&Value::Any(any))?,
                    None => String::new(),
                },
                _ => String::new(),
            };
            if !flags.contains('g') {
                return Err(RunError::Native(if kind == 4 {
                    "TypeError: String.prototype.matchAll called with a non-global RegExp argument".into()
                } else {
                    "TypeError: replaceAll must be called with a global RegExp".into()
                }));
            }
        }
        let handler = match (&arg, regex_obj) {
            (Value::Object(obj), None) => self.symbol_method(*obj, symbol.id())?,
            _ => None,
        };
        let result = if let (Some(handle), Value::Object(obj)) = (handler, &arg) {
            let mut call_args = vec![Value::Str(receiver)];
            let pass_extra = match kind {
                1 | 5 => true,
                3 => !matches!(&extra, Value::Float(n, _) if *n < 0.0) && !matches!(&extra, Value::Int(n, _) if *n < 0),
                _ => false,
            };
            if pass_extra {
                call_args.push(extra);
            }
            let call_args = self.with_this_arg(handle, *obj, call_args);
            self.invoke_function_value(handle, call_args)?
        } else {
            match kind {
                0 | 2 | 4 | 6 => {
                    let regex = match regex_obj {
                        Some(regex) => regex,
                        None => {
                            let source = match &arg {
                                Value::Void => "(?:)".to_string(),
                                other => {
                                    let text = self.dyn_to_string(other)?;
                                    if text.is_empty() { "(?:)".to_string() } else { text }
                                }
                            };
                            let flags = if kind == 4 { "g" } else { "" };
                            zun_gc::with_current_heap_mut(|heap| heap.alloc_regex_with_flags(source, flags))
                                .map_err(|error| RunError::Native(format!("SyntaxError: Invalid regular expression: {error}")))?
                        }
                    };
                    match kind {
                        0 => self.call_managed_collection("regexpMatch", &[Value::Object(regex), Value::Str(receiver)])?,
                        2 => self.call_managed_collection("regexpSearch", &[Value::Object(regex), Value::Str(receiver)])?,
                        _ => self.match_all_iterator(regex, &receiver)?,
                    }
                }
                1 | 5 => {
                    let callable = self.as_callable(&extra);
                    let all = kind == 5;
                    if let Some(regex) = regex_obj {
                        match callable {
                            Some(callback) => self.call_managed_collection(
                                "replaceFn",
                                &[Value::Object(regex), Value::Str(receiver), Value::Function(callback), Value::Bool(all)],
                            )?,
                            None => {
                                let replacement = self.dyn_to_string(&extra)?;
                                self.call_managed_collection(
                                    "regexpReplace",
                                    &[Value::Object(regex), Value::Str(receiver), Value::Str(replacement.into())],
                                )?
                            }
                        }
                    } else {
                        let needle = self.dyn_to_string(&arg)?;
                        match callable {
                            Some(callback) => self.call_managed_collection(
                                "replaceFn",
                                &[Value::Str(needle.into()), Value::Str(receiver), Value::Function(callback), Value::Bool(all)],
                            )?,
                            None => {
                                let replacement = self.dyn_to_string(&extra)?;
                                let (id, _) = zun_natives::resolve("string", if all { "replaceAll" } else { "replace" })
                                    .ok_or(RunError::BadToken)?;
                                let values = [Value::Str(receiver), Value::Str(needle.into()), Value::Str(replacement.into())];
                                let native_args: Vec<zun_natives::NativeArg> =
                                    values.iter().map(super::native::to_native_arg).collect();
                                let out = zun_natives::dispatch(id, &native_args, self.output)
                                    .map_err(|e| RunError::Native(e.0))?;
                                super::native::from_native_value(out, id)
                            }
                        }
                    }
                }
                _ => {
                    // split
                    let limit: Option<usize> = match &extra {
                        Value::Float(n, _) if *n < 0.0 => None,
                        Value::Int(n, _) if *n < 0 => None,
                        Value::Void => None,
                        other => {
                            let n = self.dyn_to_number(other)?;
                            Some(if n.is_finite() { n.trunc().rem_euclid(4294967296.0) as usize } else { 0 })
                        }
                    };
                    if let Some(regex) = regex_obj {
                        let mut call = vec![Value::Object(regex), Value::Str(receiver)];
                        match limit {
                            Some(n) => {
                                call.push(Value::Int(n as i64, zun_il::NumKind::I64));
                                self.call_managed_collection("regexpSplitLimit", &call)?
                            }
                            None => self.call_managed_collection("regexpSplit", &call)?,
                        }
                    } else {
                        let mut pieces: Vec<String> = match &arg {
                            Value::Void => vec![receiver.to_string()],
                            other => {
                                let separator = self.dyn_to_string(other)?;
                                if separator.is_empty() {
                                    zun_core::wtf16::encode(&receiver)
                                        .map(|unit| zun_core::wtf16::from_units(&[unit]))
                                        .collect()
                                } else {
                                    receiver.split(separator.as_str()).map(str::to_string).collect()
                                }
                            }
                        };
                        if let Some(limit) = limit {
                            pieces.truncate(limit);
                        }
                        let array = zun_gc::with_current_heap_mut(|heap| {
                            let array = heap.alloc_array(pieces.len());
                            for (index, piece) in pieces.iter().enumerate() {
                                let value = zun_core::Any::string(heap.intern_string(piece.as_str().into()));
                                heap.array_set(array, index, value);
                            }
                            array
                        });
                        Value::Object(array)
                    }
                }
            }
        };
        let boxed = zun_gc::with_current_heap_mut(|heap| result.box_any(heap)).unwrap_or_else(zun_core::Any::void);
        Ok(Value::Any(boxed))
    }

    /// `String.prototype.matchAll` over a (cloned) global regexp: the matches
    /// are collected through `exec` on a clone seeded with the caller's
    /// `lastIndex`, then handed out by an iterator.
    pub(super) fn match_all_iterator(&mut self, regex: zun_gc::Obj, input: &str) -> Result<Value, RunError> {
        let (source, flags, last_index) = zun_gc::with_current_heap(|heap| {
            (
                heap.regex_source(regex).unwrap_or("").to_owned(),
                heap.regex_flags(regex).unwrap_or("").to_owned(),
                heap.regex_last_index(regex).unwrap_or(0.0),
            )
        });
        let unicode = flags.contains('u') || flags.contains('v');
        let global = flags.contains('g');
        let input_units: Vec<u16> = units(input);
        let clone = zun_gc::with_current_heap_mut(|heap| {
            let clone = heap.alloc_regex_with_flags(source, flags)?;
            heap.regex_set_last_index(clone, last_index);
            Ok::<_, zun_core::RegexError>(clone)
        })
        .map_err(|error| RunError::Native(error.to_string()))?;
        let mut matches: Vec<zun_gc::Obj> = Vec::new();
        loop {
            let found = zun_gc::with_current_heap_mut(|heap| heap.regex_exec_stateful(clone, input))
                .ok_or(RunError::BadToken)?
                .map_err(|error| RunError::Native(error.to_string()))?;
            let Some(array) = found else { break };
            let empty = zun_gc::with_current_heap(|heap| {
                heap.array_get(array, 0)
                    .and_then(|any| any.as_string())
                    .is_some_and(|handle| heap.string(handle).is_empty())
            });
            if empty {
                zun_gc::with_current_heap_mut(|heap| {
                    let at = heap.regex_last_index(clone).unwrap_or(0.0).max(0.0) as usize;
                    let step = match (input_units.get(at), input_units.get(at + 1)) {
                        (Some(high), Some(low))
                            if unicode && (0xD800..=0xDBFF).contains(high) && (0xDC00..=0xDFFF).contains(low) => 2,
                        _ => 1,
                    };
                    heap.regex_set_last_index(clone, (at + step) as f64);
                });
            }
            matches.push(array);
            if !global {
                break;
            }
        }
        let iterator = zun_gc::with_current_heap_mut(|heap| {
            let outer = heap.alloc_array(matches.len());
            for (index, array) in matches.iter().enumerate() {
                heap.array_set(outer, index, zun_core::Any::object(array.addr() as usize));
            }
            let iterator = heap.alloc_array_iterator(outer)?;
            heap.ensure_dynamic_entry(iterator);
            let tag = zun_core::Any::string(heap.intern_string("RegExp String Iterator".into()));
            heap.dynamic_set_symbol(iterator, zun_core::WellKnownSymbol::ToStringTag.id(), tag);
            Some(iterator)
        })
        .ok_or(RunError::BadToken)?;
        Ok(Value::Object(iterator))
    }
}
