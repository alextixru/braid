//! `stats --host <id> --days N` (G02.4): забирает `telemetry/*.jsonl` с хоста и
//! сводит журнал админа в таблицу движок × сеть: аптайм, число переключений
//! с этого движка и их причины. Сети в журнале — хеши, не SSID; доменов там нет.
//!
//! Аптайм считается по событиям `connect` / `switch` / `network` / `disconnect`:
//! сессия движка в сети длится от её начала до следующего события, открытая —
//! до последней записи устройства. В окно попадает только часть сессии после
//! среза `now − N дней`; переключения считаются по времени события.

use crate::cli::Env;
use crate::shell::run_checked;
use crate::subd;
use anyhow::Context as _;
use braid_proto::{LogEvent, LogRecord};
use chrono::{DateTime, Utc};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cell {
    pub uptime: Duration,
    /// Переключения *с* этого движка в этой сети.
    pub switches: u32,
    pub reasons: BTreeMap<String, u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    /// (движок, сеть) → ячейка.
    pub cells: BTreeMap<(String, String), Cell>,
    /// Устройства, у которых есть события в окне.
    pub devices: usize,
    /// События в окне.
    pub events: usize,
}

/// JSONL → записи; битые строки считаются, но не роняют сводку.
pub fn parse_records(text: &str) -> (Vec<LogRecord>, usize) {
    let mut records = Vec::new();
    let mut bad = 0;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match LogRecord::from_jsonl_line(line) {
            Ok(r) => records.push(r),
            Err(_) => bad += 1,
        }
    }
    (records, bad)
}

const UNKNOWN_NETWORK: &str = "?";

struct Session {
    engine: Option<String>,
    network: Option<String>,
    since: Option<DateTime<Utc>>,
}

impl Session {
    fn accrue(&self, table: &mut Table, until: DateTime<Utc>, cutoff: DateTime<Utc>) {
        let (Some(engine), Some(since)) = (&self.engine, self.since) else {
            return;
        };
        let start = since.max(cutoff);
        if until <= start {
            return;
        }
        let network = self
            .network
            .clone()
            .unwrap_or_else(|| UNKNOWN_NETWORK.into());
        let secs = (until - start).num_seconds().max(0) as u64;
        table
            .cells
            .entry((engine.clone(), network))
            .or_default()
            .uptime += Duration::from_secs(secs);
    }
}

pub fn aggregate(files: &[(String, Vec<LogRecord>)], cutoff: DateTime<Utc>) -> Table {
    let mut table = Table::default();
    for (_device, records) in files {
        let mut sorted: Vec<&LogRecord> = records.iter().collect();
        sorted.sort_by_key(|r| r.ts);
        let in_window = sorted.iter().filter(|r| r.ts >= cutoff).count();
        if in_window > 0 {
            table.devices += 1;
            table.events += in_window;
        }
        let mut s = Session {
            engine: None,
            network: None,
            since: None,
        };
        for r in &sorted {
            match &r.event {
                LogEvent::Connect { engine, network } => {
                    s.accrue(&mut table, r.ts, cutoff);
                    if let Some(n) = network {
                        s.network = Some(n.to_string());
                    }
                    s.engine = Some(engine.to_string());
                    s.since = Some(r.ts);
                }
                LogEvent::Switch {
                    from, to, reason, ..
                } => {
                    s.accrue(&mut table, r.ts, cutoff);
                    if r.ts >= cutoff {
                        let network = s.network.clone().unwrap_or_else(|| UNKNOWN_NETWORK.into());
                        let cell = table.cells.entry((from.to_string(), network)).or_default();
                        cell.switches += 1;
                        let reason = serde_json::to_value(reason)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_owned))
                            .unwrap_or_else(|| format!("{reason:?}").to_lowercase());
                        *cell.reasons.entry(reason).or_default() += 1;
                    }
                    s.engine = Some(to.to_string());
                    s.since = Some(r.ts);
                }
                LogEvent::Disconnect { .. } => {
                    s.accrue(&mut table, r.ts, cutoff);
                    s.engine = None;
                    s.since = None;
                }
                LogEvent::Network { id, .. } => {
                    s.accrue(&mut table, r.ts, cutoff);
                    s.network = Some(id.to_string());
                    if s.engine.is_some() {
                        s.since = Some(r.ts);
                    }
                }
                _ => {}
            }
        }
        if let Some(last) = sorted.last() {
            s.accrue(&mut table, last.ts, cutoff);
        }
    }
    table
}

/// `40m`, `1h05m`, `2d03h` — без пробелов, чтобы колонки не расползались.
pub fn fmt_duration(d: Duration) -> String {
    let secs = d.as_secs();
    let (days, hours, mins) = (secs / 86_400, (secs / 3_600) % 24, (secs / 60) % 60);
    if days > 0 {
        format!("{days}d{hours:02}h")
    } else if hours > 0 {
        format!("{hours}h{mins:02}m")
    } else {
        format!("{mins}m")
    }
}

pub fn render_table(table: &Table) -> String {
    let header = ["Движок", "Сеть", "Аптайм", "Переключений", "Причины"];
    let rows: Vec<[String; 5]> = table
        .cells
        .iter()
        .map(|((engine, network), cell)| {
            let reasons = if cell.reasons.is_empty() {
                "-".to_owned()
            } else {
                cell.reasons
                    .iter()
                    .map(|(r, n)| format!("{r}:{n}"))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            [
                engine.clone(),
                network.clone(),
                fmt_duration(cell.uptime),
                cell.switches.to_string(),
                reasons,
            ]
        })
        .collect();
    let mut widths: [usize; 5] = std::array::from_fn(|i| header[i].chars().count());
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: [&str; 5]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_owned()
    };
    let mut out = line(header);
    out.push('\n');
    for row in &rows {
        out.push_str(&line(std::array::from_fn(|i| row[i].as_str())));
        out.push('\n');
    }
    out
}

pub fn stats_command(env: &mut Env<'_>, host_id: &str, days: u32) -> anyhow::Result<()> {
    env.state.require()?;
    let host = env.state.load_host(host_id)?;
    let subd_section = host
        .subd
        .as_ref()
        .with_context(|| format!("у хоста «{host_id}» нет braid-subd — `host setup {host_id}`"))?;
    let out = run_checked(env.shell, &subd::telemetry_cmd(&host, subd_section))?;
    let files = subd::parse_telemetry(&out.stdout);
    if files.is_empty() {
        writeln!(
            env.out,
            "телеметрии нет: на хосте «{host_id}» каталог telemetry/ пуст (журнал шлют только устройства admin)"
        )?;
        return Ok(());
    }
    let cutoff = env.now - chrono::Duration::days(i64::from(days));
    let mut bad = 0;
    let parsed: Vec<(String, Vec<LogRecord>)> = files
        .iter()
        .map(|(device, text)| {
            let (records, b) = parse_records(text);
            bad += b;
            (device.clone(), records)
        })
        .collect();
    let table = aggregate(&parsed, cutoff);
    writeln!(
        env.out,
        "Телеметрия «{host_id}» за {days} дн. (устройств: {}, событий: {}, битых строк: {bad})",
        table.devices, table.events
    )?;
    if table.cells.is_empty() {
        writeln!(env.out, "за период событий нет")?;
        return Ok(());
    }
    write!(env.out, "{}", render_table(&table))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_have_no_spaces() {
        assert_eq!(fmt_duration(Duration::from_secs(600)), "10m");
        assert_eq!(fmt_duration(Duration::from_secs(3900)), "1h05m");
        assert_eq!(fmt_duration(Duration::from_secs(90_000)), "1d01h");
    }
}
