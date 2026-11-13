use super::{GeneratorFrame, GeneratorState, Heap, Obj, RelocationMap};

impl Heap {
    pub fn generator_state_set(&mut self, generator: Obj, state: GeneratorState) {
        self.generator_states
            .insert(generator.addr() as usize, state);
    }
    pub fn generator_state(&self, generator: Obj) -> Option<&GeneratorState> {
        self.generator_states.get(&(generator.addr() as usize))
    }
    pub fn generator_state_mut(&mut self, generator: Obj) -> Option<&mut GeneratorState> {
        self.generator_states.get_mut(&(generator.addr() as usize))
    }
    pub fn generator_state_take(&mut self, generator: Obj) -> Option<GeneratorState> {
        self.generator_states.remove(&(generator.addr() as usize))
    }

    pub fn generator_frame_set(&mut self, generator: Obj, frame: GeneratorFrame) {
        self.generator_frames
            .insert(generator.addr() as usize, frame);
    }
    pub fn generator_frame(&self, generator: Obj) -> Option<&GeneratorFrame> {
        self.generator_frames.get(&(generator.addr() as usize))
    }
    pub fn generator_frame_mut(&mut self, generator: Obj) -> Option<&mut GeneratorFrame> {
        self.generator_frames.get_mut(&(generator.addr() as usize))
    }
    pub fn generator_frame_take(&mut self, generator: Obj) -> Option<GeneratorFrame> {
        self.generator_frames.remove(&(generator.addr() as usize))
    }
    pub(super) fn trace_generator_frame(&self, generator: Obj) -> impl Iterator<Item = usize> + '_ {
        self.generator_frames
            .get(&(generator.addr() as usize))
            .into_iter()
            .flat_map(|frame| frame.locals.iter().chain(&frame.eval_stack))
            .filter_map(|value| value.as_object())
    }

    pub(super) fn trace_generator_state(&self, generator: Obj) -> impl Iterator<Item = usize> + '_ {
        self.generator_states
            .get(&(generator.addr() as usize))
            .into_iter()
            .flat_map(|state| {
                state
                    .args
                    .iter()
                    .chain([&state.current, &state.pending_return])
                    .chain(
                        state
                            .frame
                            .iter()
                            .flat_map(|frame| frame.locals.iter().chain(&frame.eval_stack)),
                    )
            })
            .filter_map(|value| value.as_object())
    }

    /// Relocate the generator state table: both the values it holds and the
    /// address it is keyed by.
    ///
    /// Re-keying is not optional. Every address-keyed side table is re-keyed
    /// in the relocation pass, and a generator's state left under the old key
    /// makes the object stop being a generator: `generator_state` answers
    /// `None`, so the iterator protocol cannot find `next` and a `for…of` over
    /// it fails with "iterator has no next() method" partway through.
    pub(super) fn rewrite_generator_states(&mut self, relocation: &RelocationMap) -> usize {
        let mut rewritten = 0;
        let states = std::mem::take(&mut self.generator_states);
        let mut mapped = std::collections::HashMap::with_capacity(states.len());
        for (address, mut state) in states {
            let relocated = relocation.relocate_address(address);
            if relocated != address {
                rewritten += 1;
            }
            rewritten += rewrite_state_values(&mut state, relocation);
            mapped.insert(relocated, state);
        }
        self.generator_states = mapped;
        rewritten
    }

    /// The same for the suspended-frame table, keyed the same way.
    pub(super) fn rewrite_generator_frames(&mut self, relocation: &RelocationMap) -> usize {
        let mut rewritten = 0;
        let frames = std::mem::take(&mut self.generator_frames);
        let mut mapped = std::collections::HashMap::with_capacity(frames.len());
        for (address, mut frame) in frames {
            let relocated = relocation.relocate_address(address);
            if relocated != address {
                rewritten += 1;
            }
            rewritten += rewrite_frame_values(&mut frame, relocation);
            mapped.insert(relocated, frame);
        }
        self.generator_frames = mapped;
        rewritten
    }
}

fn rewrite_frame_values(frame: &mut GeneratorFrame, relocation: &RelocationMap) -> usize {
    let mut rewritten = 0;
    for value in frame.locals.iter_mut().chain(frame.eval_stack.iter_mut()) {
        if let Some(address) = value.as_object() {
            let relocated = relocation.relocate_address(address);
            if relocated != address {
                *value = zun_core::Any::object(relocated);
                rewritten += 1;
            }
        }
    }
    rewritten
}

fn rewrite_state_values(state: &mut GeneratorState, relocation: &RelocationMap) -> usize {
    let mut rewritten = 0;
    {
        let values = state
            .args
            .iter_mut()
            .chain([&mut state.current, &mut state.pending_return])
            .chain(
                state
                    .frame
                    .iter_mut()
                    .flat_map(|frame| frame.locals.iter_mut().chain(frame.eval_stack.iter_mut())),
            );
        for value in values {
            if let Some(address) = value.as_object() {
                let relocated = relocation.relocate_address(address);
                if relocated != address {
                    *value = zun_core::Any::object(relocated);
                    rewritten += 1;
                }
            }
        }
    }
    rewritten
}
