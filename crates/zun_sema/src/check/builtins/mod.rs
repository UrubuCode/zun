//! Lowering of calls into built-in objects and namespaces
//! (`Object`, `Reflect`, `Math`, `Number`, `String`, `Array`, `Promise`,
//! typed arrays, `Buffer`, `DataView`, ...).

pub(super) mod buffer;
pub(super) mod number;
pub(super) mod object;
pub(super) mod string_array;
pub(super) mod typed_array;
