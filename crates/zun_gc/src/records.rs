#[derive(Debug)]
pub(super) struct Record {
    pub(super) addr: *mut u64,
    pub(super) field_count: usize,
    pub(super) alive: bool,
    pub(super) marked: bool,
}
