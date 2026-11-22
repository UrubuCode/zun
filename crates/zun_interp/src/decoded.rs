//! Per-method decoded instruction cache for the Tier 0 dispatch loop.
//!
//! A method body is decoded once into `(Instruction, size)` pairs plus a dense
//! byte-offset -> index map, so the loop indexes instead of re-decoding. `pc`
//! stays a byte offset everywhere else (branches, handlers, OSR, generators).

use std::cell::RefCell;
use std::rc::Rc;
use zun_il::{decode_one, Instruction};

const NOT_START: u32 = u32::MAX;

pub(crate) struct DecodedBody {
    /// Copy of the code bytes; guards against a freed module's address being reused.
    code: Box<[u8]>,
    instructions: Vec<(Instruction, u32)>,
    /// Byte offset -> index into `instructions`, `NOT_START` between instructions.
    index_of: Vec<u32>,
}

impl DecodedBody {
    fn build(code: &[u8]) -> Self {
        let mut instructions = Vec::new();
        let mut index_of = vec![NOT_START; code.len()];
        let mut pc = 0;
        // Stop at the first undecodable byte: the loop falls back to `decode_one`
        // there and reports the same error as before.
        while pc < code.len() {
            let Ok((instruction, size)) = decode_one(&code[pc..]) else {
                break;
            };
            index_of[pc] = instructions.len() as u32;
            instructions.push((instruction, size as u32));
            pc += size;
        }
        Self {
            code: code.into(),
            instructions,
            index_of,
        }
    }

    /// The instruction starting at byte offset `pc`, if it is a known start.
    #[inline(always)]
    pub(crate) fn at(&self, pc: usize) -> Option<(Instruction, usize)> {
        let index = *self.index_of.get(pc)?;
        if index == NOT_START {
            return None;
        }
        let (instruction, size) = self.instructions[index as usize];
        Some((instruction, size as usize))
    }
}

type Cache = zun_gc::fast_hash::HashMap<usize, Rc<DecodedBody>>;

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::new(Cache::default());
}

/// The decoded form of `code`, which is a method body slice borrowed from a
/// module's code section. Keyed by the slice address (unique per module and
/// per method) and validated against the bytes, so several loaded modules and a
/// reused address both stay correct.
pub(crate) fn decoded_for(code: &[u8]) -> Rc<DecodedBody> {
    let key = code.as_ptr() as usize;
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(found) = cache.get(&key) {
            if *found.code == *code {
                return found.clone();
            }
        }
        let built = Rc::new(DecodedBody::build(code));
        cache.insert(key, built.clone());
        built
    })
}
