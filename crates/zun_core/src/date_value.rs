/// Millisecond-precision date value. Invalid dates carry no usable timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateValue { millis: i64, valid: bool }

impl DateValue {
    pub const fn from_millis(millis: i64) -> Self { Self { millis, valid: true } }
    pub const fn invalid() -> Self { Self { millis: 0, valid: false } }
    pub const fn is_valid(self) -> bool { self.valid }
    pub const fn millis(self) -> Option<i64> { if self.valid { Some(self.millis) } else { None } }
    pub fn compare(self, other: Self) -> Option<std::cmp::Ordering> { Some(self.millis()?.cmp(&other.millis()?)) }

    pub fn utc_date(self) -> Option<(i32, u32, u32)> {
        let days = self.millis()?.div_euclid(86_400_000);
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096).div_euclid(365);
        let year = year_of_era + era * 400;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_part = (5 * day_of_year + 2).div_euclid(153);
        let day = day_of_year - (153 * month_part + 2).div_euclid(5) + 1;
        let month = month_part + if month_part < 10 { 3 } else { -9 };
        let year = year + if month <= 2 { 1 } else { 0 };
        Some((year as i32, month as u32, day as u32))
    }
    pub fn iso_date(self) -> Option<String> { let (y,m,d) = self.utc_date()?; Some(format!("{y:04}-{m:02}-{d:02}")) }
    pub fn utc_time(self) -> Option<(u32,u32,u32,u32)> {
        let millis = self.millis()?.rem_euclid(86_400_000);
        Some(((millis/3_600_000) as u32, ((millis%3_600_000)/60_000) as u32, ((millis%60_000)/1_000) as u32, (millis%1_000) as u32))
    }
    pub fn weekday(self) -> Option<u32> { Some((self.millis()?.div_euclid(86_400_000) + 3).rem_euclid(7) as u32 + 1) }
    pub fn ordinal(self) -> Option<u32> { let (y,m,d)=self.utc_date()?; Some((1..m).map(|x| days_in_month(y,x)).sum::<u32>()+d) }
    pub fn iso_datetime(self) -> Option<String> { let (y,m,d)=self.utc_date()?; let (h,mi,s,ms)=self.utc_time()?; Some(format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z")) }
    pub fn parse_iso_datetime(value: &str) -> Option<Self> {
        let (date,time)=value.strip_suffix('Z')?.split_once('T')?; let date=Self::parse_iso_date(date)?; let mut f=time.split([':', '.']);
        let h:i64=f.next()?.parse().ok()?; let mi:i64=f.next()?.parse().ok()?; let s:i64=f.next()?.parse().ok()?; let ms:i64=f.next()?.parse().ok()?;
        if f.next().is_some() || h>23 || mi>59 || s>59 || ms>999 || h<0 || mi<0 || s<0 || ms<0 { return None; }
        Some(Self::from_millis(date.millis()?.checked_add(h*3_600_000+mi*60_000+s*1_000+ms)?))
    }
    pub fn parse_iso_date(value: &str) -> Option<Self> {
        let mut p=value.split('-'); let y:i32=p.next()?.parse().ok()?; let m:u32=p.next()?.parse().ok()?; let d:u32=p.next()?.parse().ok()?;
        if p.next().is_some() || !(1..=12).contains(&m) || d==0 || d>days_in_month(y,m) { return None; }
        Some(Self::from_millis(days_from_civil(y,m,d)?.checked_mul(86_400_000)?))
    }
}

fn days_in_month(year: i32, month: u32) -> u32 { match month { 2 if year%4==0 && (year%100!=0 || year%400==0)=>29, 2=>28, 4|6|9|11=>30, _=>31 } }
fn days_from_civil(year:i32, month:u32, day:u32)->Option<i64>{ let y=i64::from(year)-i64::from(month<=2); let era=if y>=0{y}else{y-399}.div_euclid(400); let ye=y-era*400; let m=i64::from(month); let doy=(153*(m+if m>2{-3}else{9})+2).div_euclid(5)+i64::from(day)-1; Some(era*146_097+ye*365+ye/4-ye/100+doy-719_468) }
