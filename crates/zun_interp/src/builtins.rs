//! Name resolution for native builtins implemented by managed runtime floors.

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
    ] {
        if zun_natives::resolve("zun:collections", name).map(|(candidate, _)| candidate) == Some(id)
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
