use std::sync::{Arc, Mutex};

use ironrdp_dvc::DrdynvcServer;
use ironrdp_pdu::rdp::client_info::TimezoneInfo;
use ironrdp_rdpel::server::{LocationFix, LocationHandler, LocationServer};
use ironrdp_server::DynamicChannelFactory;
use tokio::sync::watch;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SessionViewConfig {
    pub timezone: Option<TimezoneInfo>,
    pub base_zoom: f64,
}

#[derive(Clone)]
pub(crate) struct LocationInput {
    enabled: bool,
    shared: Arc<Mutex<Shared>>,
}

struct Shared {
    generation: u64,
    sender: watch::Sender<Option<LocationFix>>,
}

impl LocationInput {
    pub(crate) fn new(enabled: bool) -> (Self, watch::Receiver<Option<LocationFix>>) {
        let (sender, receiver) = watch::channel(None);
        (
            Self {
                enabled,
                shared: Arc::new(Mutex::new(Shared { generation: 0, sender })),
            },
            receiver,
        )
    }

    pub(crate) fn disconnected(&self) {
        if !self.enabled {
            return;
        }
        let mut shared = self.shared.lock().expect("location state poisoned");
        shared.generation = shared.generation.wrapping_add(1);
        let _ = shared.sender.send_replace(None);
    }
}

impl DynamicChannelFactory for LocationInput {
    fn attach(&self, channels: DrdynvcServer) -> DrdynvcServer {
        if !self.enabled {
            return channels;
        }
        let generation = {
            let mut shared = self.shared.lock().expect("location state poisoned");
            shared.generation = shared.generation.wrapping_add(1);
            let _ = shared.sender.send_replace(None);
            shared.generation
        };
        let handler = Handler {
            shared: Arc::clone(&self.shared),
            generation,
        };
        channels.with_dynamic_channel(LocationServer::new(Box::new(handler)))
    }
}

struct Handler {
    shared: Arc<Mutex<Shared>>,
    generation: u64,
}

impl LocationHandler for Handler {
    fn on_location(&mut self, fix: LocationFix) {
        let shared = self.shared.lock().expect("location state poisoned");
        if shared.generation == self.generation {
            let _ = shared.sender.send_replace(Some(fix));
        }
    }

    fn on_closed(&mut self) {
        let shared = self.shared.lock().expect("location state poisoned");
        if shared.generation == self.generation {
            let _ = shared.sender.send_replace(None);
        }
    }
}

pub(crate) fn timezone_id_from_rdp(timezone: &TimezoneInfo) -> Option<String> {
    let unix_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    timezone_id_from_rdp_at(timezone, unix_secs)
}

pub(crate) fn timezone_id_from_rdp_at(timezone: &TimezoneInfo, unix_secs: i64) -> Option<String> {
    if let Some(mapped) = match_windows_timezone(timezone) {
        return Some(mapped.to_owned());
    }
    gmt_offset_timezone_id(active_bias_minutes(timezone, unix_secs))
}

fn gmt_offset_timezone_id(total_bias: i32) -> Option<String> {
    // MS-RDPBCGR 2.2.1.11.1.1.1.1: UTC = local + active_bias (in minutes).
    if !(-840..=840).contains(&total_bias) {
        return None;
    }
    if total_bias == 0 {
        return Some("UTC".to_owned());
    }
    let east_minutes = -total_bias;
    let sign = if east_minutes >= 0 { '+' } else { '-' };
    let abs = east_minutes.unsigned_abs();
    let hours = abs / 60;
    let minutes = abs % 60;
    Some(format!("GMT{sign}{hours:02}:{minutes:02}"))
}

fn active_bias_minutes(timezone: &TimezoneInfo, unix_secs: i64) -> i32 {
    // MS-RDPBCGR 2.2.1.11.1.1.1.1: when `StandardDate.wMonth == 0` or `DaylightDate.wMonth == 0`,
    // `StandardBias` and `DaylightBias` are ignored and no DST transition occurs.
    let (Some(std_rule), Some(dst_rule)) = (&timezone.standard_date.0, &timezone.daylight_date.0) else {
        return timezone.bias;
    };
    let std_total = timezone.bias.saturating_add(timezone.standard_bias);
    let dst_total = timezone.bias.saturating_add(timezone.daylight_bias);
    if std_total == dst_total {
        return std_total;
    }
    let approx_year = civil_year_from_unix_secs(unix_secs);
    // Collect both transitions across `[approx_year - 1, approx_year, approx_year + 1]` in UTC
    // milliseconds so transitions near Jan 1 / Dec 31 boundaries (in either standard or daylight
    // local time) and both hemispheres resolve from the latest preceding transition instant.
    let mut events = [(0_i64, std_total); 6];
    let mut count = 0;
    for year in [approx_year - 1, approx_year, approx_year + 1] {
        // `daylight_date` is expressed in standard local time; `standard_date` in daylight local time.
        let Some(dst_start_ms) = transition_utc_millis(year, dst_rule, std_total) else {
            return std_total;
        };
        let Some(std_start_ms) = transition_utc_millis(year, std_rule, dst_total) else {
            return std_total;
        };
        events[count] = (dst_start_ms, dst_total);
        events[count + 1] = (std_start_ms, std_total);
        count += 2;
    }
    events[..count].sort_by_key(|&(ts, _)| ts);
    let unix_ms = unix_secs.saturating_mul(1_000);
    events[..count]
        .iter()
        .rfind(|&&(ts, _)| ts <= unix_ms)
        .map_or(std_total, |&(_, bias)| bias)
}

#[expect(
    clippy::as_conversions,
    reason = "Month, DayOfWeek, and DayOfWeekOccurrence are #[repr(u16)] enums in ironrdp-pdu"
)]
fn transition_utc_millis(
    year: i32,
    rule: &ironrdp_pdu::rdp::client_info::SystemTime,
    local_bias_minutes: i32,
) -> Option<i64> {
    let month = u32::from(rule.month as u16);
    let target_dow = u32::from(rule.day_of_week as u16); // 0 = Sunday .. 6 = Saturday
    let occurrence = u32::from(rule.day as u16); // 1..=4 or 5 = last
    if !(1..=12).contains(&month) || target_dow > 6 || !(1..=5).contains(&occurrence) {
        return None;
    }
    if rule.hour > 23 || rule.minute > 59 || rule.second > 59 || rule.milliseconds > 999 {
        return None;
    }
    let first_days = days_from_civil(year, month, 1);
    let first_dow = u32::try_from((first_days + 4).rem_euclid(7)).unwrap_or(0);
    let first_match_day = 1 + (target_dow + 7 - first_dow) % 7;
    let dim = days_in_month(year, month);
    let mut day = first_match_day + (occurrence - 1) * 7;
    if day > dim {
        day -= 7;
    }
    let local_days = days_from_civil(year, month, day);
    let local_secs =
        local_days * 86_400 + i64::from(rule.hour) * 3_600 + i64::from(rule.minute) * 60 + i64::from(rule.second);
    let utc_secs = local_secs + i64::from(local_bias_minutes) * 60;
    Some(utc_secs * 1_000 + i64::from(rule.milliseconds))
}

fn civil_year_from_unix_secs(secs: i64) -> i32 {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    i32::try_from(y + i64::from(m <= 2)).unwrap_or(1970)
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(month);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 30,
    }
}

#[derive(Clone, Copy)]
struct Rule(u16, u16, u16, u16, u16);

impl Rule {
    #[expect(
        clippy::as_conversions,
        reason = "Month, DayOfWeek, and DayOfWeekOccurrence are #[repr(u16)] enums in ironrdp-pdu"
    )]
    fn matches(self, st: &ironrdp_pdu::rdp::client_info::SystemTime) -> bool {
        (
            st.month as u16,
            st.day_of_week as u16,
            st.day as u16,
            st.hour,
            st.minute,
            st.second,
            st.milliseconds,
        ) == (self.0, self.1, self.2, self.3, self.4, 0, 0)
    }
}

fn match_windows_timezone(timezone: &TimezoneInfo) -> Option<&'static str> {
    let (iana, expected_std_bias, expected_dst) = windows_timezone_spec(timezone.standard_name.trim())?;
    match (expected_dst, &timezone.daylight_date.0, &timezone.standard_date.0) {
        (None, None, None) if timezone.bias == expected_std_bias => Some(iana),
        (Some((expected_dst_bias, dst_rule, std_rule)), Some(dst_dt), Some(std_dt))
            if timezone.bias.saturating_add(timezone.standard_bias) == expected_std_bias
                && timezone.bias.saturating_add(timezone.daylight_bias) == expected_dst_bias
                && dst_rule.matches(dst_dt)
                && std_rule.matches(std_dt) =>
        {
            Some(iana)
        }
        _ => None,
    }
}

type WindowsTzSpec = (&'static str, i32, Option<(i32, Rule, Rule)>);

fn windows_timezone_spec(name: &str) -> Option<WindowsTzSpec> {
    // Closed set of ICU zone IDs from Chromium's `windowsZones.txt` whose tzdb rules are
    // invariant Win32 nth-weekday rules across 2024..=2040. Variable-calendar zones (such as
    // Israel, Egypt, Chile, Greenland, and Morocco) or client-customized rules fall through to
    // live-evaluated `GMT±HH:MM` offsets so the transmitted MS-RDPBCGR rules are always exact.
    match name {
        "Coordinated Universal Time" | "UTC" => Some(("UTC", 0, None)),
        "AUS Central Standard Time" | "Australia/Darwin" => Some(("Australia/Darwin", -570, None)),
        "AUS Eastern Standard Time" | "Australia/Sydney" => Some((
            "Australia/Sydney",
            -600,
            Some((-660, Rule(10, 0, 1, 2, 0), Rule(4, 0, 1, 3, 0))),
        )),
        "Afghanistan Standard Time" | "Asia/Kabul" => Some(("Asia/Kabul", -270, None)),
        "Alaskan Standard Time" | "America/Anchorage" => Some((
            "America/Anchorage",
            540,
            Some((480, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Aleutian Standard Time" | "America/Adak" => Some((
            "America/Adak",
            600,
            Some((540, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Altai Standard Time" | "Asia/Barnaul" => Some(("Asia/Barnaul", -420, None)),
        "Arab Standard Time" | "Asia/Riyadh" => Some(("Asia/Riyadh", -180, None)),
        "Arabian Standard Time" | "Asia/Dubai" => Some(("Asia/Dubai", -240, None)),
        "Arabic Standard Time" | "Asia/Baghdad" => Some(("Asia/Baghdad", -180, None)),
        "Argentina Standard Time" | "America/Buenos_Aires" => Some(("America/Buenos_Aires", 180, None)),
        "Astrakhan Standard Time" | "Europe/Astrakhan" => Some(("Europe/Astrakhan", -240, None)),
        "Atlantic Standard Time" | "America/Halifax" => Some((
            "America/Halifax",
            240,
            Some((180, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Aus Central W. Standard Time" | "Australia/Eucla" => Some(("Australia/Eucla", -525, None)),
        "Azerbaijan Standard Time" | "Asia/Baku" => Some(("Asia/Baku", -240, None)),
        "Azores Standard Time" | "Atlantic/Azores" => Some((
            "Atlantic/Azores",
            60,
            Some((0, Rule(3, 0, 5, 0, 0), Rule(10, 0, 5, 1, 0))),
        )),
        "Bahia Standard Time" | "America/Bahia" => Some(("America/Bahia", 180, None)),
        "Bangladesh Standard Time" | "Asia/Dhaka" => Some(("Asia/Dhaka", -360, None)),
        "Belarus Standard Time" | "Europe/Minsk" => Some(("Europe/Minsk", -180, None)),
        "Bougainville Standard Time" | "Pacific/Bougainville" => Some(("Pacific/Bougainville", -660, None)),
        "Canada Central Standard Time" | "America/Regina" => Some(("America/Regina", 360, None)),
        "Cape Verde Standard Time" | "Atlantic/Cape_Verde" => Some(("Atlantic/Cape_Verde", 60, None)),
        "Caucasus Standard Time" | "Asia/Yerevan" => Some(("Asia/Yerevan", -240, None)),
        "Cen. Australia Standard Time" | "Australia/Adelaide" => Some((
            "Australia/Adelaide",
            -570,
            Some((-630, Rule(10, 0, 1, 2, 0), Rule(4, 0, 1, 3, 0))),
        )),
        "Central America Standard Time" | "America/Guatemala" => Some(("America/Guatemala", 360, None)),
        "Central Asia Standard Time" | "Asia/Bishkek" => Some(("Asia/Bishkek", -360, None)),
        "Central Brazilian Standard Time" | "America/Cuiaba" => Some(("America/Cuiaba", 240, None)),
        "Central Europe Standard Time" | "Europe/Budapest" => Some((
            "Europe/Budapest",
            -60,
            Some((-120, Rule(3, 0, 5, 2, 0), Rule(10, 0, 5, 3, 0))),
        )),
        "Central European Standard Time" | "Europe/Warsaw" => Some((
            "Europe/Warsaw",
            -60,
            Some((-120, Rule(3, 0, 5, 2, 0), Rule(10, 0, 5, 3, 0))),
        )),
        "Central Pacific Standard Time" | "Pacific/Guadalcanal" => Some(("Pacific/Guadalcanal", -660, None)),
        "Central Standard Time (Mexico)" | "America/Mexico_City" => Some(("America/Mexico_City", 360, None)),
        "Central Standard Time" | "America/Chicago" => Some((
            "America/Chicago",
            360,
            Some((300, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Chatham Islands Standard Time" | "Pacific/Chatham" => Some((
            "Pacific/Chatham",
            -765,
            Some((-825, Rule(9, 0, 5, 2, 45), Rule(4, 0, 1, 3, 45))),
        )),
        "China Standard Time" | "Asia/Shanghai" => Some(("Asia/Shanghai", -480, None)),
        "Dateline Standard Time" | "Etc/GMT+12" => Some(("Etc/GMT+12", 720, None)),
        "E. Africa Standard Time" | "Africa/Nairobi" => Some(("Africa/Nairobi", -180, None)),
        "E. Australia Standard Time" | "Australia/Brisbane" => Some(("Australia/Brisbane", -600, None)),
        "E. Europe Standard Time" | "Europe/Chisinau" => Some((
            "Europe/Chisinau",
            -120,
            Some((-180, Rule(3, 0, 5, 3, 0), Rule(10, 0, 5, 4, 0))),
        )),
        "E. South America Standard Time" | "America/Sao_Paulo" => Some(("America/Sao_Paulo", 180, None)),
        "Easter Island Standard Time" | "Pacific/Easter" => Some((
            "Pacific/Easter",
            360,
            Some((300, Rule(9, 6, 1, 22, 0), Rule(4, 6, 1, 22, 0))),
        )),
        "Eastern Standard Time (Mexico)" | "America/Cancun" => Some(("America/Cancun", 300, None)),
        "Eastern Standard Time" | "America/New_York" => Some((
            "America/New_York",
            300,
            Some((240, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Ekaterinburg Standard Time" | "Asia/Yekaterinburg" => Some(("Asia/Yekaterinburg", -300, None)),
        "FLE Standard Time" | "Europe/Kiev" => Some((
            "Europe/Kiev",
            -120,
            Some((-180, Rule(3, 0, 5, 3, 0), Rule(10, 0, 5, 4, 0))),
        )),
        "Fiji Standard Time" | "Pacific/Fiji" => Some(("Pacific/Fiji", -720, None)),
        "GMT Standard Time" | "Europe/London" => Some((
            "Europe/London",
            0,
            Some((-60, Rule(3, 0, 5, 1, 0), Rule(10, 0, 5, 2, 0))),
        )),
        "GTB Standard Time" | "Europe/Bucharest" => Some((
            "Europe/Bucharest",
            -120,
            Some((-180, Rule(3, 0, 5, 3, 0), Rule(10, 0, 5, 4, 0))),
        )),
        "Georgian Standard Time" | "Asia/Tbilisi" => Some(("Asia/Tbilisi", -240, None)),
        "Greenwich Standard Time" | "Atlantic/Reykjavik" => Some(("Atlantic/Reykjavik", 0, None)),
        "Haiti Standard Time" | "America/Port-au-Prince" => Some((
            "America/Port-au-Prince",
            300,
            Some((240, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Hawaiian Standard Time" | "Pacific/Honolulu" => Some(("Pacific/Honolulu", 600, None)),
        "India Standard Time" | "Asia/Calcutta" => Some(("Asia/Calcutta", -330, None)),
        "Iran Standard Time" | "Asia/Tehran" => Some(("Asia/Tehran", -210, None)),
        "Jordan Standard Time" | "Asia/Amman" => Some(("Asia/Amman", -180, None)),
        "Kaliningrad Standard Time" | "Europe/Kaliningrad" => Some(("Europe/Kaliningrad", -120, None)),
        "Korea Standard Time" | "Asia/Seoul" => Some(("Asia/Seoul", -540, None)),
        "Libya Standard Time" | "Africa/Tripoli" => Some(("Africa/Tripoli", -120, None)),
        "Line Islands Standard Time" | "Pacific/Kiritimati" => Some(("Pacific/Kiritimati", -840, None)),
        "Lord Howe Standard Time" | "Australia/Lord_Howe" => Some((
            "Australia/Lord_Howe",
            -630,
            Some((-660, Rule(10, 0, 1, 2, 0), Rule(4, 0, 1, 2, 0))),
        )),
        "Magadan Standard Time" | "Asia/Magadan" => Some(("Asia/Magadan", -660, None)),
        "Magallanes Standard Time" | "America/Punta_Arenas" => Some(("America/Punta_Arenas", 180, None)),
        "Marquesas Standard Time" | "Pacific/Marquesas" => Some(("Pacific/Marquesas", 570, None)),
        "Mauritius Standard Time" | "Indian/Mauritius" => Some(("Indian/Mauritius", -240, None)),
        "Middle East Standard Time" | "Asia/Beirut" => Some((
            "Asia/Beirut",
            -120,
            Some((-180, Rule(3, 0, 5, 0, 0), Rule(10, 0, 5, 0, 0))),
        )),
        "Montevideo Standard Time" | "America/Montevideo" => Some(("America/Montevideo", 180, None)),
        "Mountain Standard Time (Mexico)" | "America/Mazatlan" => Some(("America/Mazatlan", 420, None)),
        "Mountain Standard Time" | "America/Denver" => Some((
            "America/Denver",
            420,
            Some((360, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Myanmar Standard Time" | "Asia/Rangoon" => Some(("Asia/Rangoon", -390, None)),
        "N. Central Asia Standard Time" | "Asia/Novosibirsk" => Some(("Asia/Novosibirsk", -420, None)),
        "Namibia Standard Time" | "Africa/Windhoek" => Some(("Africa/Windhoek", -120, None)),
        "Nepal Standard Time" | "Asia/Katmandu" => Some(("Asia/Katmandu", -345, None)),
        "New Zealand Standard Time" | "Pacific/Auckland" => Some((
            "Pacific/Auckland",
            -720,
            Some((-780, Rule(9, 0, 5, 2, 0), Rule(4, 0, 1, 3, 0))),
        )),
        "Newfoundland Standard Time" | "America/St_Johns" => Some((
            "America/St_Johns",
            210,
            Some((150, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Norfolk Standard Time" | "Pacific/Norfolk" => Some((
            "Pacific/Norfolk",
            -660,
            Some((-720, Rule(10, 0, 1, 2, 0), Rule(4, 0, 1, 3, 0))),
        )),
        "North Asia East Standard Time" | "Asia/Irkutsk" => Some(("Asia/Irkutsk", -480, None)),
        "North Asia Standard Time" | "Asia/Krasnoyarsk" => Some(("Asia/Krasnoyarsk", -420, None)),
        "North Korea Standard Time" | "Asia/Pyongyang" => Some(("Asia/Pyongyang", -540, None)),
        "Omsk Standard Time" | "Asia/Omsk" => Some(("Asia/Omsk", -360, None)),
        "Pacific Standard Time (Mexico)" | "America/Tijuana" => Some((
            "America/Tijuana",
            480,
            Some((420, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Pacific Standard Time" | "America/Los_Angeles" => Some((
            "America/Los_Angeles",
            480,
            Some((420, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Pakistan Standard Time" | "Asia/Karachi" => Some(("Asia/Karachi", -300, None)),
        "Qyzylorda Standard Time" | "Asia/Qyzylorda" => Some(("Asia/Qyzylorda", -300, None)),
        "Romance Standard Time" | "Europe/Paris" => Some((
            "Europe/Paris",
            -60,
            Some((-120, Rule(3, 0, 5, 2, 0), Rule(10, 0, 5, 3, 0))),
        )),
        "Russia Time Zone 10" | "Asia/Srednekolymsk" => Some(("Asia/Srednekolymsk", -660, None)),
        "Russia Time Zone 11" | "Asia/Kamchatka" => Some(("Asia/Kamchatka", -720, None)),
        "Russia Time Zone 3" | "Europe/Samara" => Some(("Europe/Samara", -240, None)),
        "Russian Standard Time" | "Europe/Moscow" => Some(("Europe/Moscow", -180, None)),
        "SA Eastern Standard Time" | "America/Cayenne" => Some(("America/Cayenne", 180, None)),
        "SA Pacific Standard Time" | "America/Bogota" => Some(("America/Bogota", 300, None)),
        "SA Western Standard Time" | "America/La_Paz" => Some(("America/La_Paz", 240, None)),
        "SE Asia Standard Time" | "Asia/Bangkok" => Some(("Asia/Bangkok", -420, None)),
        "Saint Pierre Standard Time" | "America/Miquelon" => Some((
            "America/Miquelon",
            180,
            Some((120, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "Sakhalin Standard Time" | "Asia/Sakhalin" => Some(("Asia/Sakhalin", -660, None)),
        "Samoa Standard Time" | "Pacific/Apia" => Some(("Pacific/Apia", -780, None)),
        "Sao Tome Standard Time" | "Africa/Sao_Tome" => Some(("Africa/Sao_Tome", 0, None)),
        "Saratov Standard Time" | "Europe/Saratov" => Some(("Europe/Saratov", -240, None)),
        "Singapore Standard Time" | "Asia/Singapore" => Some(("Asia/Singapore", -480, None)),
        "South Africa Standard Time" | "Africa/Johannesburg" => Some(("Africa/Johannesburg", -120, None)),
        "South Sudan Standard Time" | "Africa/Juba" => Some(("Africa/Juba", -120, None)),
        "Sri Lanka Standard Time" | "Asia/Colombo" => Some(("Asia/Colombo", -330, None)),
        "Sudan Standard Time" | "Africa/Khartoum" => Some(("Africa/Khartoum", -120, None)),
        "Syria Standard Time" | "Asia/Damascus" => Some(("Asia/Damascus", -180, None)),
        "Taipei Standard Time" | "Asia/Taipei" => Some(("Asia/Taipei", -480, None)),
        "Tasmania Standard Time" | "Australia/Hobart" => Some((
            "Australia/Hobart",
            -600,
            Some((-660, Rule(10, 0, 1, 2, 0), Rule(4, 0, 1, 3, 0))),
        )),
        "Tocantins Standard Time" | "America/Araguaina" => Some(("America/Araguaina", 180, None)),
        "Tokyo Standard Time" | "Asia/Tokyo" => Some(("Asia/Tokyo", -540, None)),
        "Tomsk Standard Time" | "Asia/Tomsk" => Some(("Asia/Tomsk", -420, None)),
        "Tonga Standard Time" | "Pacific/Tongatapu" => Some(("Pacific/Tongatapu", -780, None)),
        "Transbaikal Standard Time" | "Asia/Chita" => Some(("Asia/Chita", -540, None)),
        "Turkey Standard Time" | "Europe/Istanbul" => Some(("Europe/Istanbul", -180, None)),
        "Turks And Caicos Standard Time" | "America/Grand_Turk" => Some((
            "America/Grand_Turk",
            300,
            Some((240, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "US Eastern Standard Time" | "America/Indianapolis" => Some((
            "America/Indianapolis",
            300,
            Some((240, Rule(3, 0, 2, 2, 0), Rule(11, 0, 1, 2, 0))),
        )),
        "US Mountain Standard Time" | "America/Phoenix" => Some(("America/Phoenix", 420, None)),
        "UTC+12" | "Etc/GMT-12" => Some(("Etc/GMT-12", -720, None)),
        "UTC+13" | "Etc/GMT-13" => Some(("Etc/GMT-13", -780, None)),
        "UTC-02" | "Etc/GMT+2" => Some(("Etc/GMT+2", 120, None)),
        "UTC-08" | "Etc/GMT+8" => Some(("Etc/GMT+8", 480, None)),
        "UTC-09" | "Etc/GMT+9" => Some(("Etc/GMT+9", 540, None)),
        "UTC-11" | "Etc/GMT+11" => Some(("Etc/GMT+11", 660, None)),
        "Ulaanbaatar Standard Time" | "Asia/Ulaanbaatar" => Some(("Asia/Ulaanbaatar", -480, None)),
        "Venezuela Standard Time" | "America/Caracas" => Some(("America/Caracas", 240, None)),
        "Vladivostok Standard Time" | "Asia/Vladivostok" => Some(("Asia/Vladivostok", -600, None)),
        "Volgograd Standard Time" | "Europe/Volgograd" => Some(("Europe/Volgograd", -180, None)),
        "W. Australia Standard Time" | "Australia/Perth" => Some(("Australia/Perth", -480, None)),
        "W. Central Africa Standard Time" | "Africa/Lagos" => Some(("Africa/Lagos", -60, None)),
        "W. Europe Standard Time" | "Europe/Berlin" => Some((
            "Europe/Berlin",
            -60,
            Some((-120, Rule(3, 0, 5, 2, 0), Rule(10, 0, 5, 3, 0))),
        )),
        "W. Mongolia Standard Time" | "Asia/Hovd" => Some(("Asia/Hovd", -420, None)),
        "West Asia Standard Time" | "Asia/Tashkent" => Some(("Asia/Tashkent", -300, None)),
        "West Pacific Standard Time" | "Pacific/Port_Moresby" => Some(("Pacific/Port_Moresby", -600, None)),
        "Yakutsk Standard Time" | "Asia/Yakutsk" => Some(("Asia/Yakutsk", -540, None)),
        "Yukon Standard Time" | "America/Whitehorse" => Some(("America/Whitehorse", 420, None)),
        _ => None,
    }
}

pub(crate) fn base_zoom_for_scale(desktop_scale_factor: Option<u32>) -> f64 {
    let Some(scale) = desktop_scale_factor.filter(|&s| (100..=500).contains(&s)) else {
        return 0.0;
    };
    if scale == 100 {
        return 0.0;
    }
    (f64::from(scale) / 100.0).ln() / 1.2_f64.ln()
}

#[cfg(test)]
mod tests {
    use super::*;

    // oracle: spec MS-RDPBCGR §2.2.1.11.1.1.1.1 (`UTC = local + Bias`); mutant inverting GMT sign or rejecting non-ASCII/unmapped offsets.
    #[test]
    fn rdp_timezone_maps_windows_names_iana_ids_and_gmt_offsets() {
        use ironrdp_pdu::rdp::client_info::{DayOfWeek, DayOfWeekOccurrence, Month, OptionalSystemTime, SystemTime};

        let jul_15_2026 = 1_784_073_600; // 2026-07-15T00:00:00Z
        let jan_15_2026 = 1_768_435_200; // 2026-01-15T00:00:00Z

        // Canonical US Eastern DST rules -> maps to IANA `America/New_York`.
        let eastern = TimezoneInfo {
            bias: 300,
            standard_name: "Eastern Standard Time".to_owned(),
            standard_bias: 0,
            daylight_bias: -60,
            daylight_date: OptionalSystemTime(Some(SystemTime {
                month: Month::March,
                day_of_week: DayOfWeek::Sunday,
                day: DayOfWeekOccurrence::Second,
                hour: 2,
                minute: 0,
                second: 0,
                milliseconds: 0,
            })),
            standard_date: OptionalSystemTime(Some(SystemTime {
                month: Month::November,
                day_of_week: DayOfWeek::Sunday,
                day: DayOfWeekOccurrence::First,
                hour: 2,
                minute: 0,
                second: 0,
                milliseconds: 0,
            })),
            daylight_name: "Eastern Daylight Time".to_owned(),
        };
        assert_eq!(
            timezone_id_from_rdp_at(&eastern, jul_15_2026).as_deref(),
            Some("America/New_York")
        );

        // Same `standard_name`, but client disabled DST (`standard_date = None`) and left a stale
        // `standard_bias = -60`: MS-RDPBCGR §2.2.1.11.1.1.1.1 requires ignoring `standard_bias`
        // and keeping fixed `GMT-05:00` even in July.
        let eastern_no_dst = TimezoneInfo {
            bias: 300,
            standard_name: "Eastern Standard Time".to_owned(),
            standard_bias: -60,
            ..TimezoneInfo::default()
        };
        assert_eq!(
            timezone_id_from_rdp_at(&eastern_no_dst, jul_15_2026).as_deref(),
            Some("GMT-05:00")
        );

        let phoenix = TimezoneInfo {
            bias: 420,
            standard_name: "US Mountain Standard Time".to_owned(),
            ..TimezoneInfo::default()
        };
        assert_eq!(
            timezone_id_from_rdp_at(&phoenix, jul_15_2026).as_deref(),
            Some("America/Phoenix")
        );

        // An IANA-looking `standard_name` without matching DST rules must NOT bypass MS-RDPBCGR's
        // transmitted fixed bias (`GMT-05:00`), whereas `"America/New_York"` WITH matching Eastern
        // DST rules resolves to `"America/New_York"`.
        let iana_fixed = TimezoneInfo {
            bias: 300,
            standard_name: "America/Montreal".to_owned(),
            ..TimezoneInfo::default()
        };
        assert_eq!(
            timezone_id_from_rdp_at(&iana_fixed, jul_15_2026).as_deref(),
            Some("GMT-05:00")
        );
        let mut iana_eastern = eastern.clone();
        iana_eastern.standard_name = "America/New_York".to_owned();
        assert_eq!(
            timezone_id_from_rdp_at(&iana_eastern, jul_15_2026).as_deref(),
            Some("America/New_York")
        );

        // Non-zero transition seconds prevent false canonical IANA equivalence and evaluate the
        // client's exact rule instead.
        let mut custom_seconds = eastern;
        custom_seconds.daylight_date.0.as_mut().unwrap().second = 30;
        assert_eq!(
            timezone_id_from_rdp_at(&custom_seconds, jul_15_2026).as_deref(),
            Some("GMT-04:00")
        );

        // Bias +300 minutes = UTC-5 -> ICU `GMT-05:00`; Bias -330 minutes = UTC+5:30 -> ICU `GMT+05:30`.
        let custom_west = TimezoneInfo {
            bias: 300,
            standard_name: "Custom Zone".to_owned(),
            ..TimezoneInfo::default()
        };
        assert_eq!(timezone_id_from_rdp(&custom_west).as_deref(), Some("GMT-05:00"));

        let custom_east = TimezoneInfo {
            bias: -330,
            standard_name: String::new(),
            ..TimezoneInfo::default()
        };
        assert_eq!(timezone_id_from_rdp(&custom_east).as_deref(), Some("GMT+05:30"));

        // Variable-calendar zones like Israel evaluate the client's exact transmitted rule to
        // `GMT+02:00` in winter and `GMT+03:00` in summer rather than assuming fixed-weekday parity.
        let israel = TimezoneInfo {
            bias: -120,
            standard_name: "Israel Standard Time".to_owned(),
            standard_bias: 0,
            daylight_bias: -60,
            daylight_date: OptionalSystemTime(Some(SystemTime {
                month: Month::March,
                day_of_week: DayOfWeek::Friday,
                day: DayOfWeekOccurrence::Last,
                hour: 2,
                minute: 0,
                second: 0,
                milliseconds: 0,
            })),
            standard_date: OptionalSystemTime(Some(SystemTime {
                month: Month::October,
                day_of_week: DayOfWeek::Sunday,
                day: DayOfWeekOccurrence::Last,
                hour: 2,
                minute: 0,
                second: 0,
                milliseconds: 0,
            })),
            daylight_name: "Jerusalem Daylight Time".to_owned(),
        };
        assert_eq!(
            timezone_id_from_rdp_at(&israel, jan_15_2026).as_deref(),
            Some("GMT+02:00")
        );
        assert_eq!(
            timezone_id_from_rdp_at(&israel, jul_15_2026).as_deref(),
            Some("GMT+03:00")
        );

        // Unmapped custom DST zone: UTC-5 standard, UTC-4 daylight (2nd Sun Mar 02:00 -> 1st Sun Nov 02:00).
        let custom_dst = TimezoneInfo {
            bias: 300,
            standard_name: "Synthetic Atlantic Custom".to_owned(),
            standard_bias: 0,
            daylight_bias: -60,
            daylight_date: OptionalSystemTime(Some(SystemTime {
                month: Month::March,
                day_of_week: DayOfWeek::Sunday,
                day: DayOfWeekOccurrence::Second,
                hour: 2,
                minute: 0,
                second: 0,
                milliseconds: 0,
            })),
            standard_date: OptionalSystemTime(Some(SystemTime {
                month: Month::November,
                day_of_week: DayOfWeek::Sunday,
                day: DayOfWeekOccurrence::First,
                hour: 2,
                minute: 0,
                second: 0,
                milliseconds: 0,
            })),
            daylight_name: String::new(),
        };
        assert_eq!(
            timezone_id_from_rdp_at(&custom_dst, jan_15_2026).as_deref(),
            Some("GMT-05:00")
        );
        assert_eq!(
            timezone_id_from_rdp_at(&custom_dst, jul_15_2026).as_deref(),
            Some("GMT-04:00")
        );

        // Year-boundary Southern-hemisphere rule where standard time starts Jan 1 (1st Thu) 00:00
        // daylight time (`2026-01-01T00:00+01:00 == 2025-12-31T23:00Z`): at `2025-12-31T23:30Z`
        // (still 2025 in standard local time!), standard time (`UTC`) must already be in effect.
        let new_year_boundary = TimezoneInfo {
            bias: 0,
            standard_name: "Custom Boundary".to_owned(),
            standard_bias: 0,
            daylight_bias: -60,
            daylight_date: OptionalSystemTime(Some(SystemTime {
                month: Month::October,
                day_of_week: DayOfWeek::Sunday,
                day: DayOfWeekOccurrence::First,
                hour: 2,
                minute: 0,
                second: 0,
                milliseconds: 0,
            })),
            standard_date: OptionalSystemTime(Some(SystemTime {
                month: Month::January,
                day_of_week: DayOfWeek::Thursday,
                day: DayOfWeekOccurrence::First,
                hour: 0,
                minute: 0,
                second: 0,
                milliseconds: 0,
            })),
            daylight_name: String::new(),
        };
        let dec_31_2025_2230z = 1_767_220_200; // 2025-12-31T22:30:00Z (before transition -> GMT+01:00)
        let dec_31_2025_2330z = 1_767_223_800; // 2025-12-31T23:30:00Z (after transition -> UTC)
        assert_eq!(
            timezone_id_from_rdp_at(&new_year_boundary, dec_31_2025_2230z).as_deref(),
            Some("GMT+01:00")
        );
        assert_eq!(
            timezone_id_from_rdp_at(&new_year_boundary, dec_31_2025_2330z).as_deref(),
            Some("UTC")
        );
    }

    // oracle: Chromium logarithmic zoom scale uses base 1.2 (`1.2^zoom == scale / 100`).
    #[test]
    fn desktop_scale_factor_maps_to_chromium_logarithmic_zoom() {
        assert_eq!(base_zoom_for_scale(None), 0.0);
        assert_eq!(base_zoom_for_scale(Some(100)), 0.0);
        let zoom_200 = base_zoom_for_scale(Some(200));
        assert!((1.2_f64.powf(zoom_200) - 2.0).abs() < 1e-9);
    }
}
