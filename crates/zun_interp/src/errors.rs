use super::Value;

#[derive(Debug, Clone)]
pub enum RunError {
    NoEntryPoint,
    BadToken,
    Malformed(String),
    Native(String),
    Thrown(Value),
    /// An execution failure annotated with the nearest sequence point.
    SourceFrame {
        cause: Box<RunError>,
        method: String,
        source_span: Option<(u32, u32)>,
    },
    /// Internal control flow used to transfer a suspended frame to the
    /// resumable interpreter boundary.
    Suspended,
}

/// The readable form of a thrown value: `TypeError: message` for an error
/// object, the text for a string. A bare `Object(Obj(0x...))` tells whoever
/// reads the process's stderr nothing at all, and the heap is reachable from
/// here through its thread-local accessor.
thread_local! {
    /// The last thrown object's rendered text, keyed by its address.
    ///
    /// `Display` for [`RunError::Thrown`] needs the heap to build
    /// `name: message`, and the driver prints after execution ends, when the
    /// `HeapScope` has dropped -- so eight conformance fixtures reported a bare
    /// `Object(Obj(0x...))` and could not be diagnosed at all. The text is
    /// therefore rendered at the interpreter's terminal boundary, while the
    /// heap is alive, and left here for `Display` to find.
    ///
    /// It is a side channel rather than a different `RunError` variant on
    /// purpose: collapsing `Thrown` into `Native` is observable, and four
    /// tests match on `Thrown(..)` -- `finally_runs_before_an_unhandled_rethrow`
    /// and `import_dynamic_il_rejects_unknown_modules` among them. Keyed by
    /// address so a stale entry cannot be attributed to a different throw.
    static THROWN_TEXT: std::cell::RefCell<Option<(usize, String)>> =
        const { std::cell::RefCell::new(None) };
}

/// Remember `text` as the rendered form of the thrown object at `address`.
pub fn remember_thrown_text(address: usize, text: String) {
    THROWN_TEXT.with(|slot| *slot.borrow_mut() = Some((address, text)));
}

/// The remembered text for `address`, if it is the one most recently rendered.
fn remembered_thrown_text(address: usize) -> Option<String> {
    THROWN_TEXT.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|(remembered, _)| *remembered == address)
            .map(|(_, text)| text.clone())
    })
}

fn thrown_text(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.to_string()),
        Value::Object(_) | Value::Any(_) => {
            let object = match value {
                Value::Object(object) => Some(*object),
                Value::Any(any) => any
                    .as_object()
                    .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) }),
                _ => None,
            }?;
            // The remembered text first: at `Display` time the heap is usually
            // gone, and it was rendered at the interpreter boundary precisely
            // for this moment.
            if let Some(text) = remembered_thrown_text(object.addr() as usize) {
                return Some(text);
            }
            let heap = zun_gc::current_heap_ptr()?;
            let heap = unsafe { &*heap };
            {
                let name = heap
                    .error_name(object)
                    .map(str::to_owned)
                    .or_else(|| dynamic_text(heap, object, "name"))?;
                let message = heap
                    .error_message(object)
                    .map(str::to_owned)
                    .or_else(|| dynamic_text(heap, object, "message"))
                    .unwrap_or_default();
                Some(if message.is_empty() {
                    name
                } else {
                    format!("{name}: {message}")
                })
            }
        }
        _ => None,
    }
}

/// A string-valued dynamic property set directly on this object.
fn own_dynamic_text(heap: &zun_gc::Heap, object: zun_gc::Obj, key: &str) -> Option<String> {
    let handle = heap.dynamic_get(object, key)?.as_string()?;
    Some(heap.try_string(handle)?.to_string())
}
/// A string-valued property, searched along the prototype chain.
///
/// Own-only was not enough: an `Error` subclass carries `name` on its
/// prototype, so the lookup answered `None`, `thrown_text` gave up on its `?`,
/// and eight fixtures printed a bare `Object(Obj(0x...))` instead of the error.
fn dynamic_text(heap: &zun_gc::Heap, object: zun_gc::Obj, key: &str) -> Option<String> {
    let mut current = Some(object);
    for _ in 0..32 {
        let object = current?;
        if let Some(text) = own_dynamic_text(heap, object, key) {
            return Some(text);
        }
        current = heap
            .dynamic_prototype(object)
            .and_then(|any| any.as_object())
            .filter(|address| *address != 0)
            .map(|address| unsafe { zun_gc::Obj::from_addr(address as *mut u64) });
    }
    None
}


impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RunError::NoEntryPoint => write!(f, "module has no entry point"),
            RunError::Malformed(m) => write!(f, "malformed bytecode: {m}"),
            RunError::Native(m) => write!(f, "{m}"),
            RunError::BadToken => write!(f, "call did not resolve to a method"),
            RunError::Thrown(value) => match thrown_text(value) {
                Some(text) => write!(f, "unhandled exception: {text}"),
                None => write!(f, "unhandled exception: {value:?}"),
            },
            RunError::SourceFrame {
                cause,
                method,
                source_span,
            } => {
                write!(f, "{cause} at {method}")?;
                if let Some((start, end)) = source_span {
                    write!(f, " [{start}..{end}]")?;
                }
                Ok(())
            }
            RunError::Suspended => write!(f, "execution suspended"),
        }
    }
}

impl RunError {
    pub(crate) fn annotate(self, module: &zun_module::Module, method_rid: u32, pc: u32) -> Self {
        if matches!(
            self,
            RunError::SourceFrame { .. } | RunError::Suspended | RunError::Thrown(_)
        ) {
            return self;
        }
        let Some(frame) = module.debug_frame_at(method_rid, pc) else {
            return self;
        };
        Self::SourceFrame {
            cause: Box::new(self),
            method: frame.method_name.to_owned(),
            source_span: frame.source_span,
        }
    }

    /// Render an error with the source frame associated with the failing
    /// method and program counter. This keeps the existing error variants
    /// stable while allowing hosts to consume `.zm` sequence-point metadata.
    pub fn with_source_frame(
        &self,
        module: &zun_module::Module,
        method_rid: u32,
        pc: u32,
    ) -> String {
        let Some(frame) = module.debug_frame_at(method_rid, pc) else {
            return self.to_string();
        };
        match frame.source_span {
            Some((start, end)) => format!("{} at {} [{}..{}]", self, frame.method_name, start, end),
            None => format!("{} at {}", self, frame.method_name),
        }
    }
}

impl std::error::Error for RunError {}

#[cfg(test)]
mod tests {
    use super::*;
    use zun_metadata::MethodDef;

    #[test]
    fn source_frame_formatting_consumes_sequence_points() {
        let mut module = zun_module::Module::new();
        let name = module.strings.intern("main");
        module.tables.add_method_def(MethodDef {
            name,
            signature: zun_metadata::BlobIndex(0),
            body_offset: u32::MAX,
            is_static: true,
        });
        module.sequence_points.push(zun_module::SequencePoint {
            method_rid: 1,
            pc: 4,
            source_start: 12,
            source_end: 20,
        });
        let error = RunError::BadToken;
        assert_eq!(
            error.with_source_frame(&module, 1, 5),
            "call did not resolve to a method at main [12..20]"
        );
        assert_eq!(
            error.annotate(&module, 1, 5).to_string(),
            "call did not resolve to a method at main [12..20]"
        );
    }
}
