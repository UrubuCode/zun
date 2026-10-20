use super::{Heap, Obj};

impl Heap {
    pub fn date_millis(&self, object: Obj) -> Option<i64> {
        self.dates.get(&(object.addr() as usize))?.millis()
    }
    pub fn date_is_valid(&self, object: Obj) -> Option<bool> {
        Some(self.dates.get(&(object.addr() as usize))?.is_valid())
    }
    pub fn date_set_millis(&mut self, object: Obj, millis: i64) -> Option<()> {
        *self.dates.get_mut(&(object.addr() as usize))? = zun_core::DateValue::from_millis(millis);
        Some(())
    }
    pub fn date_invalidate(&mut self, object: Obj) -> Option<()> {
        *self.dates.get_mut(&(object.addr() as usize))? = zun_core::DateValue::invalid();
        Some(())
    }
    pub fn date_compare(&self, left: Obj, right: Obj) -> Option<std::cmp::Ordering> {
        self.dates.get(&(left.addr() as usize))?.compare(*self.dates.get(&(right.addr() as usize))?)
    }
    pub fn date_utc_date(&self, object: Obj) -> Option<(i32, u32, u32)> {
        self.dates.get(&(object.addr() as usize))?.utc_date()
    }
    pub fn date_utc_time(&self, object: Obj) -> Option<(u32, u32, u32, u32)> {
        self.dates.get(&(object.addr() as usize))?.utc_time()
    }
    pub fn date_weekday(&self, object: Obj) -> Option<u32> {
        self.dates.get(&(object.addr() as usize))?.weekday()
    }
    pub fn date_ordinal(&self, object: Obj) -> Option<u32> {
        self.dates.get(&(object.addr() as usize))?.ordinal()
    }
    pub fn date_iso(&self, object: Obj) -> Option<String> {
        self.dates.get(&(object.addr() as usize))?.iso_date()
    }
    pub fn date_iso_datetime(&self, object: Obj) -> Option<String> {
        self.dates.get(&(object.addr() as usize))?.iso_datetime()
    }
}
