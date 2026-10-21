use super::{GeneratorFrame, Heap, Obj, RelocationMap};

impl Heap {
    pub fn generator_frame_set(&mut self, generator: Obj, frame: GeneratorFrame) {
        self.generator_frames.insert(generator.addr() as usize, frame);
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
        self.generator_frames.get(&(generator.addr() as usize)).into_iter()
            .flat_map(|frame| frame.locals.iter().chain(&frame.eval_stack))
            .filter_map(|value| value.as_object())
    }
    pub(super) fn rewrite_generator_frames(&mut self, relocation: &RelocationMap) -> usize {
        let mut rewritten = 0;
        for frame in self.generator_frames.values_mut() {
            for value in frame.locals.iter_mut().chain(frame.eval_stack.iter_mut()) {
                if let Some(address) = value.as_object() {
                    let relocated = relocation.relocate_address(address);
                    if relocated != address { *value = zun_core::Any::object(relocated); rewritten += 1; }
                }
            }
        }
        rewritten
    }
}
