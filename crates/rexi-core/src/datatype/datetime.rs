//! Date-Time ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime), Tables 7-3
//! and 7-4).
//!
//! A date or time is sent as a row of small parts (components), each packed into a
//! fixed number of bits. Which parts appear, and in what order, depends on the XML
//! Schema type ("kind"). A part in square brackets is only sent when the presence bit
//! just before it is 1. The text forms are those of XML Schema 1.0 Part 2, for
//! example [`dateTime`](https://www.w3.org/TR/xmlschema-2/#dateTime),
//! [`time`](https://www.w3.org/TR/xmlschema-2/#time) and
//! [`gYearMonth`](https://www.w3.org/TR/xmlschema-2/#gYearMonth).
//!
//! Components, in order, per kind. A part in square brackets is there only when the
//! Boolean "presence" bit before it is 1 (the spec's notation for optional parts).
//!
//! | kind       | components                                                   |
//! |------------|--------------------------------------------------------------|
//! | gYear      | Year, presence, \[TimeZone\]                                 |
//! | gYearMonth | Year, MonthDay, presence, \[TimeZone\]                       |
//! | date       | Year, MonthDay, presence, \[TimeZone\]                       |
//! | dateTime   | Year, MonthDay, Time, presence, \[FractionalSecs\], presence, \[TimeZone\] |
//! | gMonth     | MonthDay, presence, \[TimeZone\]                             |
//! | gMonthDay  | MonthDay, presence, \[TimeZone\]                             |
//! | gDay       | MonthDay, presence, \[TimeZone\]                             |
//! | time       | Time, presence, \[FractionalSecs\], presence, \[TimeZone\]   |
//!
//! * Year: Integer, `year - 2000`.
//! * MonthDay: 9-bit, `month * 32 + day`. Day needs 5 bits (1..=31) and month 4 bits
//!   (1..=12), so `* 32` puts the month above the day. A missing part is 0 (gMonth
//!   and gYearMonth: `month * 32`, gDay: `day`), as EXIficient does.
//! * Time: 17-bit, `((hour * 64) + minute) * 64 + second`. Each of minute and second
//!   gets 6 bits (0..=63) and hour gets the top 5 bits.
//! * FractionalSecs: presence bit, then the digits after the point, reversed, as an
//!   Unsigned Integer (reversing keeps leading zeros, as in Decimal). The part is left
//!   out when it is zero ([Canonical EXI](https://www.w3.org/TR/exi-c14n/), and
//!   EXIficient).
//! * TimeZone: presence bit, then 11-bit `tzHours * 64 + tzMinutes + 896`. Hours and
//!   minutes carry the offset's sign, and adding 896 (= 14 * 64) makes the smallest
//!   offset, -14:00, come out as 0. The [`DateTime::timezone`] field holds minutes
//!   from UTC; the `hours * 64 + minutes` form is only used in the stream.
//!
//! Text handling works the same as EXIficient's `DateTimeValue` (Canonical EXI rules:
//! "The Hour value MUST NOT be 24",
//! [`DateTimeValue.java#L84-L111`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/values/DateTimeValue.java#L84-L111);
//! "Fractional seconds component MUST be omitted if its value is zero",
//! [`#L118-L120`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/values/DateTimeValue.java#L118-L120); "per default 'Z'",
//! [`#L670-L674`](https://github.com/EXIficient/exificient-core/blob/9d853cb456a9d5557fa9d4544917a894c576ab96/src/main/java/com/siemens/ct/exi/core/values/DateTimeValue.java#L670-L674)).
//!
//! - On output a zero offset prints as `Z` (so `+00:00` becomes `Z`).
//! - Fractional seconds print without zeros at the end (`.0120` becomes `.012`) and
//!   are left out when zero.
//! - Years print with at least four digits.
//! - An hour of 24 is changed to 00 of the next day
//!   (`1999-12-31T24:00:00Z` -> `2000-01-01T00:00:00Z`).
//! - Parsing accepts the old `--MM--` gMonth form.
//!
//! EXIficient's handling of hour 24 only moves on to the next month at December 31
//! (it produces day 0 otherwise); this code moves to the next month at every month
//! end. Ranges are checked here (EXIficient does not check them): month 1..=12, day
//! 1..=31, hour 0..=24, minute 0..=59, second 0..=60 (60 allows a leap second), time
//! zone within ±14:00.

use crate::io::{ByteSink, ByteSource};
use crate::prelude::*;

use super::integer::{read_integer, write_integer};
use super::unsigned::{read_unsigned, write_unsigned};
use crate::error::{ExiError, ExiResult};
use crate::io::{BitReader, BitWriter};
use crate::value::{DateTime, DateTimeKind, Int};

const NAME: &str = "Date-Time";
/// The Year field holds `year - YEAR_OFFSET`
/// ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime)).
pub const YEAR_OFFSET: i64 = 2000;
/// The TimeZone field holds `hours * 64 + minutes + TIMEZONE_OFFSET` in 11 bits, so
/// that negative offsets become positive numbers
/// ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime)).
pub const TIMEZONE_OFFSET: i64 = 896;
const BITS_MONTH_DAY: u8 = 9;
const BITS_TIME: u8 = 17;
const BITS_TIMEZONE: u8 = 11;
const MAX_TIMEZONE_MINUTES: i16 = 14 * 60;

impl DateTimeKind {
    /// Whether this kind has a Year field.
    pub const fn has_year(self) -> bool {
        matches!(
            self,
            Self::DateTime | Self::Date | Self::GYear | Self::GYearMonth
        )
    }

    /// Whether this kind has the combined MonthDay field.
    pub const fn has_month_day(self) -> bool {
        !matches!(self, Self::GYear | Self::Time)
    }

    /// Whether the text form of this kind has a month.
    pub const fn has_month(self) -> bool {
        !matches!(self, Self::GYear | Self::Time | Self::GDay)
    }

    /// Whether the text form of this kind has a day.
    pub const fn has_day(self) -> bool {
        matches!(
            self,
            Self::DateTime | Self::Date | Self::GMonthDay | Self::GDay
        )
    }

    /// Whether this kind has a Time field (and so may have fractional seconds).
    pub const fn has_time(self) -> bool {
        matches!(self, Self::DateTime | Self::Time)
    }

    /// The XSD type name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::DateTime => "dateTime",
            Self::Date => "date",
            Self::Time => "time",
            Self::GYear => "gYear",
            Self::GYearMonth => "gYearMonth",
            Self::GMonth => "gMonth",
            Self::GMonthDay => "gMonthDay",
            Self::GDay => "gDay",
        }
    }
}

impl DateTime {
    /// Parses the XSD text form of `kind` (no spaces around it).
    pub fn from_lexical(kind: DateTimeKind, s: &str) -> Option<Self> {
        let mut c = Cursor { s, pos: 0 };
        let mut dt = DateTime::default();
        match kind {
            DateTimeKind::GYear => dt.year = c.year()?,
            DateTimeKind::GYearMonth => {
                dt.year = c.year()?;
                c.expect(b'-')?;
                dt.month = c.two_digits()?;
            }
            DateTimeKind::Date => {
                dt.year = c.year()?;
                c.expect(b'-')?;
                dt.month = c.two_digits()?;
                c.expect(b'-')?;
                dt.day = c.two_digits()?;
            }
            DateTimeKind::DateTime => {
                dt.year = c.year()?;
                c.expect(b'-')?;
                dt.month = c.two_digits()?;
                c.expect(b'-')?;
                dt.day = c.two_digits()?;
                c.expect(b'T')?;
                c.time(&mut dt)?;
            }
            DateTimeKind::Time => c.time(&mut dt)?,
            DateTimeKind::GMonth => {
                c.expect(b'-')?;
                c.expect(b'-')?;
                dt.month = c.two_digits()?;
                // Old gMonth form `--MM--`: skip the trailing `--` if present.
                if c.s[c.pos..].starts_with("--") {
                    c.pos += 2;
                }
            }
            DateTimeKind::GMonthDay => {
                c.expect(b'-')?;
                c.expect(b'-')?;
                dt.month = c.two_digits()?;
                c.expect(b'-')?;
                dt.day = c.two_digits()?;
            }
            DateTimeKind::GDay => {
                c.expect(b'-')?;
                c.expect(b'-')?;
                c.expect(b'-')?;
                dt.day = c.two_digits()?;
            }
        }
        dt.timezone = c.timezone()?;
        if c.pos != c.s.len() {
            return None;
        }
        dt.check(kind).ok()?;
        dt.normalize_hour24(kind);
        Some(dt)
    }

    /// Standard text form of `kind`, as EXIficient's `DateTimeValue.toString` prints
    /// it.
    pub fn to_lexical(&self, kind: DateTimeKind) -> String {
        let mut s = String::with_capacity(32);
        match kind {
            DateTimeKind::GYear => push_year(&mut s, self.year),
            DateTimeKind::GYearMonth => {
                push_year(&mut s, self.year);
                push_two(&mut s, '-', self.month);
            }
            DateTimeKind::Date => {
                push_year(&mut s, self.year);
                push_two(&mut s, '-', self.month);
                push_two(&mut s, '-', self.day);
            }
            DateTimeKind::DateTime => {
                push_year(&mut s, self.year);
                push_two(&mut s, '-', self.month);
                push_two(&mut s, '-', self.day);
                s.push('T');
                self.push_time(&mut s);
            }
            DateTimeKind::Time => self.push_time(&mut s),
            DateTimeKind::GMonth => {
                s.push('-');
                push_two(&mut s, '-', self.month);
            }
            DateTimeKind::GMonthDay => {
                s.push('-');
                push_two(&mut s, '-', self.month);
                push_two(&mut s, '-', self.day);
            }
            DateTimeKind::GDay => {
                s.push_str("--");
                push_two(&mut s, '-', self.day);
            }
        }
        match self.timezone {
            None => {}
            Some(0) => s.push('Z'),
            Some(tz) => {
                s.push(if tz < 0 { '-' } else { '+' });
                let mag = tz.unsigned_abs();
                s.push_str(&format!("{:02}:{:02}", mag / 60, mag % 60));
            }
        }
        s
    }

    fn push_time(&self, s: &mut String) {
        s.push_str(&format!(
            "{:02}:{:02}:{:02}",
            self.hour, self.minute, self.second
        ));
        if let Some(frac) = self.fractional_secs.filter(|f| *f != 0) {
            s.push('.');
            s.extend(frac.to_string().chars().rev());
        }
    }

    /// Checks the ranges of the parts that `kind` uses. Parts it does not use are
    /// ignored.
    pub fn check(&self, kind: DateTimeKind) -> Result<(), String> {
        if kind.has_month() && !(1..=12).contains(&self.month) {
            return Err(format!("month {} out of range", self.month));
        }
        if kind.has_day() && !(1..=31).contains(&self.day) {
            return Err(format!("day {} out of range", self.day));
        }
        if kind.has_time() {
            if self.hour > 24 {
                return Err(format!("hour {} out of range", self.hour));
            }
            if self.minute > 59 {
                return Err(format!("minute {} out of range", self.minute));
            }
            if self.second > 60 {
                return Err(format!("second {} out of range", self.second));
            }
        }
        if let Some(tz) = self.timezone
            && tz.abs() > MAX_TIMEZONE_MINUTES
        {
            return Err(format!("time zone offset {tz} minutes out of range"));
        }
        Ok(())
    }

    /// Canonical EXI: the hour must not be 24, so move on to 00:xx of the next day.
    /// Only `dateTime` has a date to move forward; for `time` the hour just becomes
    /// 0. Leap years follow the Gregorian rule (every 4 years, but not every 100,
    /// except every 400).
    pub fn normalize_hour24(&mut self, kind: DateTimeKind) {
        if !kind.has_time() || self.hour != 24 {
            return;
        }
        self.hour = 0;
        if kind != DateTimeKind::DateTime {
            return;
        }
        self.day += 1;
        if self.day > days_in_month(self.year, self.month) {
            self.day = 1;
            self.month += 1;
            if self.month > 12 {
                self.month = 1;
                self.year += 1;
            }
        }
    }
}

fn days_in_month(year: i64, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            if leap { 29 } else { 28 }
        }
        _ => 31,
    }
}

fn push_year(s: &mut String, year: i64) {
    if year < 0 {
        s.push('-');
    }
    s.push_str(&format!("{:04}", year.unsigned_abs()));
}

fn push_two(s: &mut String, sep: char, v: u8) {
    s.push(sep);
    s.push_str(&format!("{v:02}"));
}

struct Cursor<'a> {
    s: &'a str,
    pos: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.as_bytes().get(self.pos).copied()
    }

    fn expect(&mut self, c: u8) -> Option<()> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }

    fn digit(&self, at: usize) -> Option<u8> {
        self.s
            .as_bytes()
            .get(at)
            .filter(|b| b.is_ascii_digit())
            .map(|b| b - b'0')
    }

    fn two_digits(&mut self) -> Option<u8> {
        let v = self.digit(self.pos)? * 10 + self.digit(self.pos + 1)?;
        self.pos += 2;
        Some(v)
    }

    fn digits(&mut self) -> &str {
        let start = self.pos;
        while self.digit(self.pos).is_some() {
            self.pos += 1;
        }
        &self.s[start..self.pos]
    }

    /// `-YYYY` or `YYYY`, with at least four digits (XSD allows more).
    fn year(&mut self) -> Option<i64> {
        let negative = self.expect(b'-').is_some();
        let digits = self.digits();
        if digits.len() < 4 {
            return None;
        }
        let v: i64 = digits.parse().ok()?;
        Some(if negative { -v } else { v })
    }

    /// `hh:mm:ss`, then optionally `.` and one or more digits. The fraction digits are
    /// stored reversed, and a fraction of all zeros is stored as `None`.
    fn time(&mut self, dt: &mut DateTime) -> Option<()> {
        dt.hour = self.two_digits()?;
        self.expect(b':')?;
        dt.minute = self.two_digits()?;
        self.expect(b':')?;
        dt.second = self.two_digits()?;
        if self.expect(b'.').is_some() {
            let digits = self.digits();
            if digits.is_empty() {
                return None;
            }
            let reversed: String = digits.chars().rev().collect();
            let frac: u64 = reversed.parse().ok()?;
            dt.fractional_secs = if frac == 0 { None } else { Some(frac) };
        }
        Some(())
    }

    /// `Z`, `(+|-)hh:mm`, or nothing. Returns `None` on a malformed offset.
    fn timezone(&mut self) -> Option<Option<i16>> {
        match self.peek() {
            None => Some(None),
            Some(b'Z') => {
                self.pos += 1;
                Some(Some(0))
            }
            Some(sign @ (b'+' | b'-')) => {
                self.pos += 1;
                let hours = self.two_digits()?;
                self.expect(b':')?;
                let minutes = self.two_digits()?;
                if minutes > 59 {
                    return None;
                }
                let total = i16::from(hours) * 60 + i16::from(minutes);
                Some(Some(if sign == b'-' { -total } else { total }))
            }
            Some(_) => None,
        }
    }
}

/// The 9-bit MonthDay part: `month * 32 + day`, with the part a kind lacks set to 0.
fn month_day(kind: DateTimeKind, dt: &DateTime) -> u64 {
    match kind {
        DateTimeKind::GYearMonth | DateTimeKind::GMonth => u64::from(dt.month) * 32,
        DateTimeKind::GDay => u64::from(dt.day),
        _ => u64::from(dt.month) * 32 + u64::from(dt.day),
    }
}

/// Minutes from UTC -> the 11-bit value in the stream (`hours * 64 + minutes + 896`).
/// The sign goes on the whole `hours * 64 + minutes` value, so -00:50 is `896 - 50`.
fn timezone_to_wire(tz: i16) -> u64 {
    let mag = i64::from(tz.unsigned_abs());
    let base64 = (mag / 60) * 64 + mag % 60;
    (if tz < 0 { -base64 } else { base64 } + TIMEZONE_OFFSET) as u64
}

/// The 11-bit time zone value from the stream -> minutes from UTC. Undoes
/// [`timezone_to_wire`]: take away 896, then split the size into `hours * 64` and
/// `minutes`.
fn timezone_from_wire(wire: u64) -> ExiResult<i16> {
    let signed = wire as i64 - TIMEZONE_OFFSET;
    let mag = signed.unsigned_abs();
    let minutes = (mag / 64) * 60 + mag % 64;
    let minutes =
        i16::try_from(minutes).map_err(|_| ExiError::value(NAME, "time zone out of range"))?;
    Ok(if signed < 0 { -minutes } else { minutes })
}

/// Writes the parts `kind` uses, in the order of Table 7-4
/// ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime)). The value is checked
/// first and an hour of 24 is moved to the next day, as EXIficient's constructor does.
pub fn write_datetime<W: ByteSink>(
    w: &mut BitWriter<W>,
    kind: DateTimeKind,
    value: &DateTime,
) -> ExiResult<()> {
    value
        .check(kind)
        .map_err(|reason| ExiError::value(NAME, reason))?;
    let mut dt = *value;
    dt.normalize_hour24(kind);
    if kind.has_year() {
        let offset = dt
            .year
            .checked_sub(YEAR_OFFSET)
            .ok_or_else(|| ExiError::value(NAME, "year out of range"))?;
        write_integer(w, &Int::Small(offset))?;
    }
    if kind.has_month_day() {
        w.write_nbit_unsigned(month_day(kind, &dt), BITS_MONTH_DAY)?;
    }
    if kind.has_time() {
        let time = (u64::from(dt.hour) * 64 + u64::from(dt.minute)) * 64 + u64::from(dt.second);
        w.write_nbit_unsigned(time, BITS_TIME)?;
        match dt.fractional_secs {
            Some(frac) if frac != 0 => {
                w.write_bool(true)?;
                write_unsigned(w, frac)?;
            }
            _ => w.write_bool(false)?,
        }
    }
    match dt.timezone {
        Some(tz) => {
            w.write_bool(true)?;
            w.write_nbit_unsigned(timezone_to_wire(tz), BITS_TIMEZONE)
        }
        None => w.write_bool(false),
    }
}

/// Reads the parts `kind` uses, in the order of Table 7-4
/// ([EXI 7.1.8](https://www.w3.org/TR/exi/#encodingDateTime)). The bit fields can hold
/// more than a date allows (hour 31, minute 63, month 15), so the same range check
/// as on the write side refuses those. An hour of 24 in the stream is moved to 00 of
/// the next day, the same as on the write side.
pub fn read_datetime<R: ByteSource>(
    r: &mut BitReader<R>,
    kind: DateTimeKind,
) -> ExiResult<DateTime> {
    let mut dt = DateTime::default();
    if kind.has_year() {
        dt.year = read_integer(r)?
            .to_i64()
            .and_then(|y| y.checked_add(YEAR_OFFSET))
            .ok_or_else(|| ExiError::value(NAME, "year out of range"))?;
    }
    if kind.has_month_day() {
        let md = r.read_nbit_unsigned(BITS_MONTH_DAY)?;
        dt.month = (md / 32) as u8;
        dt.day = (md % 32) as u8;
    }
    if kind.has_time() {
        let time = r.read_nbit_unsigned(BITS_TIME)?;
        dt.hour = (time / (64 * 64)) as u8;
        dt.minute = ((time / 64) % 64) as u8;
        dt.second = (time % 64) as u8;
        if r.read_bool()? {
            let frac = read_unsigned(r)?;
            dt.fractional_secs = if frac == 0 { None } else { Some(frac) };
        }
    }
    if r.read_bool()? {
        dt.timezone = Some(timezone_from_wire(r.read_nbit_unsigned(BITS_TIMEZONE)?)?);
    }
    dt.check(kind)
        .map_err(|reason| ExiError::value(NAME, reason))?;
    dt.normalize_hour24(kind);
    Ok(dt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use DateTimeKind as K;

    fn enc(aligned: bool, kind: K, dt: &DateTime) -> Vec<u8> {
        let mut w = BitWriter::with_alignment(Vec::new(), aligned);
        write_datetime(&mut w, kind, dt).unwrap_or_default();
        w.finish().unwrap_or_default()
    }

    fn round_trip(aligned: bool, kind: K, dt: &DateTime) -> String {
        let bytes = enc(aligned, kind, dt);
        let mut r = BitReader::with_alignment(bytes.as_slice(), aligned);
        read_datetime(&mut r, kind)
            .map(|d| d.to_lexical(kind))
            .unwrap_or_default()
    }

    fn check(kind: K, input: &str, expected: &str) {
        let dt = DateTime::from_lexical(kind, input);
        assert!(dt.is_some(), "{input}");
        let dt = dt.unwrap_or_default();
        assert_eq!(dt.to_lexical(kind), expected, "{input}");
        assert_eq!(round_trip(false, kind, &dt), expected, "{input} bit");
        assert_eq!(round_trip(true, kind, &dt), expected, "{input} byte");
    }

    #[test]
    fn exificient_datetime_core_test() {
        // DatetimeCoreTest: every lexical input and its expected decoded string.
        check(K::GYear, "2007", "2007");
        check(K::GYear, "-0007", "-0007");
        check(K::GYear, "2001Z", "2001Z");
        check(K::GYear, "2001+05:00", "2001+05:00");
        check(K::GYear, "2009-13:59", "2009-13:59");
        check(K::GYearMonth, "1999-10", "1999-10");
        check(K::GYearMonth, "2007-10", "2007-10");
        check(K::GYearMonth, "1999-10-12:11", "1999-10-12:11");
        check(K::GYearMonth, "1809-10+00:33", "1809-10+00:33");
        check(K::GYearMonth, "1809-02Z", "1809-02Z");
        check(K::Date, "2002-10-11+13:00", "2002-10-11+13:00");
        check(K::Date, "0100-10-11-00:50", "0100-10-11-00:50");
        check(K::DateTime, "0001-01-01T01:03:12", "0001-01-01T01:03:12");
        check(K::DateTime, "2005-01-01T00:00:00Z", "2005-01-01T00:00:00Z");
        check(
            K::DateTime,
            "1979-01-01T00:00:00.0120",
            "1979-01-01T00:00:00.012",
        );
        check(
            K::DateTime,
            "0001-01-01T00:00:00.111+00:33",
            "0001-01-01T00:00:00.111+00:33",
        );
        check(K::DateTime, "2009-04-01T12:34:56Z", "2009-04-01T12:34:56Z");
        check(K::GMonth, "--12", "--12");
        check(K::GMonth, "--07Z", "--07Z");
        check(K::GMonth, "--07+12:34", "--07+12:34");
        check(K::GMonth, "--07--+09:00", "--07+09:00");
        check(K::GMonthDay, "--01-28", "--01-28");
        check(K::GMonthDay, "--10-17+00:00", "--10-17Z");
        check(K::GDay, "---16", "---16");
        check(K::Time, "12:34:56", "12:34:56");
        check(K::Time, "12:34:56.135", "12:34:56.135");
        check(K::Time, "12:34:56.135-12:56", "12:34:56.135-12:56");
        // testDatetimeFail1
        assert_eq!(DateTime::from_lexical(K::Time, "12:34:XXX"), None);
    }

    #[test]
    fn exificient_hour_24_normalisation() {
        // testDatetimeEquals2 / Normalize1 / Equals6: constructor rolls hour 24 over.
        let dt =
            DateTime::from_lexical(K::DateTime, "2015-08-11T24:00:00-07:30").unwrap_or_default();
        assert_eq!(
            (dt.month, dt.day, dt.hour, dt.minute, dt.second),
            (8, 12, 0, 0, 0)
        );
        check(
            K::DateTime,
            "2015-08-11T24:00:00-07:30",
            "2015-08-12T00:00:00-07:30",
        );
        check(K::DateTime, "2015-08-11T24:00:00", "2015-08-12T00:00:00");
        check(K::DateTime, "1999-12-31T24:00:00Z", "2000-01-01T00:00:00Z");
        // Month-end roll-over (EXIficient would produce day 0 here).
        check(K::DateTime, "2015-08-31T24:00:00", "2015-09-01T00:00:00");
        check(K::DateTime, "2012-02-28T24:00:00", "2012-02-29T00:00:00");
        check(K::DateTime, "2013-02-28T24:00:00", "2013-03-01T00:00:00");
        check(K::Time, "24:00:00", "00:00:00");
        // The decoder fixes hour 24 too: 2015-08-11T24:00:00 in the stream.
        let raw = DateTime {
            year: 2015,
            month: 8,
            day: 11,
            hour: 24,
            ..DateTime::default()
        };
        let mut w = BitWriter::new(Vec::new());
        write_integer(&mut w, &Int::Small(15)).unwrap_or_default();
        w.write_nbit_unsigned(8 * 32 + 11, 9).unwrap_or_default();
        w.write_nbit_unsigned(24 * 64 * 64, 17).unwrap_or_default();
        w.write_bool(false).unwrap_or_default();
        w.write_bool(false).unwrap_or_default();
        let bytes = w.finish().unwrap_or_default();
        // The encoder fixes hour 24 first (as EXIficient's constructor does), so it
        // never produces this stream; it encodes the raw value the same as the fixed
        // one.
        let normalised = DateTime {
            year: 2015,
            month: 8,
            day: 12,
            hour: 0,
            ..DateTime::default()
        };
        assert_ne!(bytes, enc(false, K::DateTime, &raw));
        assert_eq!(
            enc(false, K::DateTime, &raw),
            enc(false, K::DateTime, &normalised)
        );
        let mut r = BitReader::new(bytes.as_slice());
        assert_eq!(
            read_datetime(&mut r, K::DateTime).map(|d| d.to_lexical(K::DateTime)),
            Ok("2015-08-12T00:00:00".into())
        );
    }

    #[test]
    fn task_examples_and_components() {
        check(K::DateTime, "2026-04-15T17:45:00Z", "2026-04-15T17:45:00Z");
        check(
            K::DateTime,
            "2001-10-26T21:32:52.12679+02:00",
            "2001-10-26T21:32:52.12679+02:00",
        );
        check(K::GMonth, "--05", "--05");
        check(K::GDay, "---12", "---12");
        check(K::GYearMonth, "2001-10", "2001-10");
        check(K::GYear, "2001", "2001");
        check(K::GYear, "12345", "12345");
        check(K::GYear, "-12345Z", "-12345Z");
        check(
            K::DateTime,
            "2001-10-26T21:32:52.000",
            "2001-10-26T21:32:52",
        );
        check(
            K::DateTime,
            "2001-10-26T21:32:52.5",
            "2001-10-26T21:32:52.5",
        );
        check(
            K::DateTime,
            "2001-10-26T21:32:52.05",
            "2001-10-26T21:32:52.05",
        );
        check(
            K::DateTime,
            "2001-10-26T21:32:52.50",
            "2001-10-26T21:32:52.5",
        );
        check(K::Time, "23:59:60", "23:59:60");
        let dt = DateTime::from_lexical(K::DateTime, "2001-10-26T21:32:52.12679+02:00")
            .unwrap_or_default();
        assert_eq!(dt.fractional_secs, Some(97621));
        assert_eq!(dt.timezone, Some(120));
        let dt = DateTime::from_lexical(K::GDay, "---12").unwrap_or_default();
        assert_eq!((dt.month, dt.day), (0, 12));
        let dt = DateTime::from_lexical(K::GMonth, "--05").unwrap_or_default();
        assert_eq!((dt.month, dt.day), (5, 0));
    }

    #[test]
    fn invalid_lexical_forms() {
        for (kind, s) in [
            (K::GYear, "207"),
            (K::GYear, "2007-"),
            (K::GYear, "2007+5:00"),
            (K::GYear, "2007+05:00x"),
            (K::GYear, "2007+15:00"),
            (K::GYear, "2007+05:60"),
            (K::GYear, " 2007"),
            (K::GYearMonth, "2007-13"),
            (K::GYearMonth, "2007-00"),
            (K::Date, "2007-01-32"),
            (K::Date, "2007-01-00"),
            (K::Date, "2007-01-01T"),
            (K::DateTime, "2007-01-01 10:00:00"),
            (K::DateTime, "2007-01-01T25:00:00"),
            (K::DateTime, "2007-01-01T10:60:00"),
            (K::DateTime, "2007-01-01T10:00:61"),
            (K::DateTime, "2007-01-01T10:00:00."),
            (K::Time, "12:34"),
            (K::Time, "12:34:56:78"),
            (K::GMonth, "-12"),
            (K::GMonth, "--12-"),
            (K::GMonthDay, "--12"),
            (K::GDay, "--12"),
            (K::GDay, "---1"),
        ] {
            assert_eq!(DateTime::from_lexical(kind, s), None, "{kind:?} {s:?}");
        }
    }

    #[test]
    fn wire_layout() {
        // gYear "2001Z": sign 0, varint(1), presence 1, 896 in 11 bits.
        let dt = DateTime::from_lexical(K::GYear, "2001Z").unwrap_or_default();
        assert_eq!(enc(false, K::GYear, &dt), vec![0x00, 0xDC, 0x00]);
        assert_eq!(enc(true, K::GYear, &dt), vec![0x00, 0x01, 0x01, 0x80, 0x03]);
        // Time zone wire values: hours * 64 + minutes + 896.
        assert_eq!(timezone_to_wire(0), 896);
        assert_eq!(timezone_to_wire(33), 929);
        assert_eq!(timezone_to_wire(-839), 5);
        assert_eq!(timezone_to_wire(840), 896 + 896);
        assert_eq!(timezone_to_wire(-840), 0);
        for tz in [-840i16, -839, -50, -1, 0, 1, 33, 120, 839, 840] {
            assert_eq!(timezone_from_wire(timezone_to_wire(tz)), Ok(tz));
        }
        // gDay "---16": 9-bit 16, presence 0.
        let dt = DateTime::from_lexical(K::GDay, "---16").unwrap_or_default();
        assert_eq!(enc(false, K::GDay, &dt), vec![0b0000_1000, 0b0000_0000]);
        assert_eq!(enc(true, K::GDay, &dt), vec![16, 0, 0]);
        // time "12:34:56.135": 17-bit ((12*64)+34)*64+56 = 51384, presence 1, varint(531), presence 0.
        let dt = DateTime::from_lexical(K::Time, "12:34:56.135").unwrap_or_default();
        assert_eq!(
            enc(true, K::Time, &dt),
            vec![0xB8, 0xC8, 0x00, 0x01, 0x93, 0x04, 0x00]
        );
    }

    #[test]
    fn typed_values_are_validated() {
        let mut w = BitWriter::new(Vec::new());
        let bad = DateTime {
            month: 13,
            day: 1,
            ..DateTime::default()
        };
        assert!(write_datetime(&mut w, K::Date, &bad).is_err());
        let bad = DateTime {
            month: 1,
            day: 1,
            timezone: Some(841),
            ..DateTime::default()
        };
        assert!(write_datetime(&mut w, K::Date, &bad).is_err());
        // Fields a kind does not use are ignored.
        let odd = DateTime {
            year: 2001,
            month: 99,
            day: 99,
            hour: 99,
            ..DateTime::default()
        };
        assert!(write_datetime(&mut w, K::GYear, &odd).is_ok());
        // The reader checks the same ranges: time "31:60:09" fits the 17-bit field
        // (((31 * 64) + 60) * 64 + 9) but is no time (found by fuzzing).
        let mut w = BitWriter::new(Vec::new());
        let _ = w.write_nbit_unsigned((31 * 64 + 60) * 64 + 9, BITS_TIME);
        let _ = w.write_bool(false);
        let _ = w.write_bool(false);
        let bytes = w.finish().unwrap_or_default();
        assert!(read_datetime(&mut BitReader::new(bytes.as_slice()), K::Time).is_err());
        let zero_frac = DateTime {
            hour: 1,
            fractional_secs: Some(0),
            ..DateTime::default()
        };
        let none_frac = DateTime {
            hour: 1,
            fractional_secs: None,
            ..DateTime::default()
        };
        assert_eq!(
            enc(false, K::Time, &zero_frac),
            enc(false, K::Time, &none_frac)
        );
    }
}
