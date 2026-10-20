//! Name resolution for native builtins implemented by managed runtime floors.

use super::native::{from_native_value, to_native_arg};
use super::stack::pop_n;
use super::{Interp, RunError, Value};

pub(super) fn call_builtin(
    interp: &mut Interp<'_>,
    id: i32,
    stack: &mut Vec<Value>,
) -> Result<(), RunError> {
    let id = id as u32;
    let args = pop_n(stack, zun_natives::param_count(id))?;
    if let Some(name) = managed_collection_builtin(id).or_else(|| managed_buffer_builtin(id)) {
        stack.push(interp.call_managed_collection(name, &args)?);
        return Ok(());
    }
    let native_args: Vec<zun_natives::NativeArg> = args.iter().map(to_native_arg).collect();
    let result = zun_natives::dispatch(id, &native_args, interp.output)
        .map_err(|e| RunError::Native(e.0))?;
    stack.push(from_native_value(result, id));
    Ok(())
}

pub(crate) fn managed_collection_builtin(id: u32) -> Option<&'static str> {
    for name in [
        "bufferNew",
        "bufferLength",
        "bufferGetU8",
        "bufferSetU8",
        "mapNew",
        "mapSet",
        "mapGet",
        "mapHas",
        "mapSize",
        "mapDelete",
        "mapClear",
        "mapValues",
        "mapKeys",
        "mapEntries",
        "setNew",
        "setAdd",
        "setHas",
        "setSize",
        "setDelete",
        "setClear",
        "setValues",
        "setEntries",
        "weakMapNew",
        "weakMapSet",
        "weakMapGet",
        "weakMapHas",
        "weakMapDelete",
        "promiseNew",
        "promiseResolve",
        "promiseReject",
        "promiseState",
        "promiseResult",
    ] {
        if zun_natives::resolve("zun:collections", name)
            .or_else(|| zun_natives::resolve("zun:promise", name))
            .map(|(candidate, _)| candidate)
            == Some(id)
        {
            return Some(name);
        }
    }
    None
}

pub(crate) fn managed_buffer_builtin(id: u32) -> Option<&'static str> {
    for name in [
        "bufferNew",
        "bufferLength",
        "bufferGetU8",
        "bufferSetU8",
        "bufferReadU32",
        "bufferWriteU32",
        "bufferReadF64",
        "bufferWriteF64",
        "dataViewNew",
        "dataViewGetU32",
        "dataViewSetU32",
        "dataViewGetU16",
        "dataViewSetU16",
        "dataViewGetF64",
        "dataViewSetF64",
        "u32ArrayNew",
        "u32ArrayGet",
        "u32ArraySet",
        "u8ArrayNew",
        "u8ArrayGet",
        "u8ArraySet",
        "f64ArrayNew",
        "f64ArrayGet",
        "f64ArraySet",
        "u16ArrayNew",
        "u16ArrayGet",
        "u16ArraySet",
        "arrayIteratorNew",
        "arrayIteratorNext",
        "arrayIteratorDone",
        "setIteratorNew",
        "setIteratorNext",
        "setIteratorDone",
        "mapIteratorNew",
        "mapIteratorNext",
        "mapIteratorDone",
    ] {
        if zun_natives::resolve("zun:buffer", name).map(|(candidate, _)| candidate) == Some(id) {
            return Some(name);
        }
    }
    None
}
