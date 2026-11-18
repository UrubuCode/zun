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
            // The heap must still be registered here. It usually is NOT at
            // `Display` time -- the driver prints after execution ends and the
            // `HeapScope` has dropped -- which is why `RunError::materialize_thrown`
            // renders the text at the interpreter's terminal boundary instead.
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
    /// Render a thrown value's `name: message` **while the heap is still
    /// registered**, so the text survives to printing time.
    ///
    /// `Display` for [`RunError::Thrown`] reads the heap, but the driver prints
    /// the error once execution is over, by which point the `HeapScope` has
    /// dropped and `current_heap_ptr` answers `None`. Every uncaught thrown
    /// object therefore printed as `Object(Obj(0x...))`, which made three
    /// conformance fixtures impossible to diagnose. Two earlier attempts at
    /// this looked for the wrong thing -- the escaping path, and the catch
    /// handling -- because the failure is not *where* the error travels but
    /// *when* it is formatted. One env-gated trace in `thrown_text` named the
    /// branch (`no heap registered`) in a single run.
    ///
    /// Called only from `Interp::annotate_error`, whose every caller is a
    /// `return Err(..)` at a terminal boundary in `runner.rs`: the heap is
    /// alive there and the error can no longer be caught, so collapsing the
    /// variant cannot change any catch decision. `Native` displays its string
    /// verbatim, so the output is exactly what `Thrown` would have printed had
    /// the heap still been there.
    pub fn materialize_thrown(self) -> Self {
        match &self {
            // Only a value whose text needs the heap. A `Value::Str` already
            // displays correctly without one, and collapsing it would change
            // the variant a caller can observe -- `import_dynamic_il_rejects_unknown_modules`
            // matches `Thrown(Value::Str(..))` and caught exactly that.
            RunError::Thrown(value @ (Value::Object(_) | Value::Any(_))) => match thrown_text(value) {
                Some(text) => RunError::Native(format!("unhandled exception: {text}")),
                None => self,
            },
            _ => self,
        }
    }

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
