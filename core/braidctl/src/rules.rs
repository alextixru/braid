//! `rules check`: проверка `state/rules/*.json` и `state/modes.json` (ист. 49, 66).
//!
//! Проверяется всё, что иначе всплыло бы у клиента: домены и подстановки
//! (`x.y`, `*.x.y`), подсети IPv4 (IPv6 — правило 9, биты хоста — опечатка),
//! исключения, совпадение `id` с именем файла, ссылки режимов на наборы,
//! ровно один `default`. Каждая проблема — файл, строка, причина; строка
//! находится по тексту элемента в файле.

use crate::cli::Env;
use crate::state::State;
use braid_proto::{DomainPattern, Mode, RuleSet};
use ipnet::{Ipv4Net, Ipv6Net};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Относительно `state/`: `rules/cursor.json`, `modes.json`.
    pub file: String,
    pub line: usize,
    pub reason: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.file, self.line, self.reason)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub issues: Vec<Issue>,
    pub sets: Vec<RuleSet>,
    pub modes: Vec<Mode>,
}

impl CheckReport {
    pub fn ok(&self) -> bool {
        self.issues.is_empty()
    }
}

/// Проверка без вывода — для `publish`, который не выкладывает битые правила.
pub fn check(state: &State) -> anyhow::Result<CheckReport> {
    let mut report = CheckReport::default();
    let mut set_ids = BTreeSet::new();
    for path in state.rule_files()? {
        let rel = format!(
            "rules/{}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        let text = fs::read_to_string(&path)?;
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if let Some(set) = check_rule_file(&rel, &stem, &text, &mut report.issues) {
            set_ids.insert(set.id.clone());
            report.sets.push(set);
        }
    }
    report.sets.sort_by(|a, b| a.id.cmp(&b.id));

    let modes_path = state.modes_path();
    match fs::read_to_string(&modes_path) {
        Ok(text) => {
            report.modes = check_modes_file("modes.json", &text, &set_ids, &mut report.issues);
        }
        Err(e) => report.issues.push(Issue {
            file: "modes.json".into(),
            line: 1,
            reason: format!("не читается: {e}"),
        }),
    }
    Ok(report)
}

pub fn check_command(env: &mut Env<'_>) -> anyhow::Result<()> {
    env.state.require()?;
    let report = check(&env.state)?;
    for issue in &report.issues {
        writeln!(env.out, "{issue}")?;
    }
    writeln!(
        env.out,
        "наборов: {}, режимов: {}, ошибок: {}",
        report.sets.len(),
        report.modes.len(),
        report.issues.len()
    )?;
    if report.ok() {
        Ok(())
    } else {
        anyhow::bail!("правила не прошли проверку — исправь и повтори");
    }
}

/// Номер строки (с 1) n-го вхождения `needle`; если не нашли — 1.
pub fn line_of(text: &str, needle: &str, nth: usize) -> usize {
    let mut from = 0;
    let mut seen = 0;
    while let Some(pos) = text[from..].find(needle) {
        let at = from + pos;
        seen += 1;
        if seen == nth {
            return text[..at].matches('\n').count() + 1;
        }
        from = at + needle.len().max(1);
    }
    1
}

fn quoted(item: &str) -> String {
    serde_json::to_string(item).unwrap_or_else(|_| format!("\"{item}\""))
}

fn parse_json(rel: &str, text: &str, issues: &mut Vec<Issue>) -> Option<Value> {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => Some(v),
        Err(e) => {
            issues.push(Issue {
                file: rel.to_owned(),
                line: e.line().max(1),
                reason: format!("не JSON: {e}"),
            });
            None
        }
    }
}

fn check_rule_file(rel: &str, stem: &str, text: &str, issues: &mut Vec<Issue>) -> Option<RuleSet> {
    let value = parse_json(rel, text, issues)?;
    let Some(obj) = value.as_object() else {
        issues.push(Issue {
            file: rel.into(),
            line: 1,
            reason: "ожидался объект { id, name, domains, ips, … }".into(),
        });
        return None;
    };
    let before = issues.len();
    match obj.get("id").and_then(Value::as_str) {
        Some(id) if id == stem => {}
        Some(id) => issues.push(Issue {
            file: rel.into(),
            line: line_of(text, &quoted(id), 1),
            reason: format!("id «{id}» не совпадает с именем файла «{stem}»"),
        }),
        None => issues.push(Issue {
            file: rel.into(),
            line: 1,
            reason: "нет поля id".into(),
        }),
    }
    match obj.get("name").and_then(Value::as_str) {
        Some(name) if !name.trim().is_empty() => {}
        _ => issues.push(Issue {
            file: rel.into(),
            line: 1,
            reason: "нет поля name".into(),
        }),
    }
    check_domains(rel, text, obj.get("domains"), "domains", issues);
    check_domains(
        rel,
        text,
        obj.get("exclude_domains"),
        "exclude_domains",
        issues,
    );
    check_subnets(rel, text, obj.get("ips"), "ips", issues);
    check_subnets(rel, text, obj.get("exclude_ips"), "exclude_ips", issues);
    if issues.len() != before {
        return None;
    }
    match serde_json::from_value::<RuleSet>(value) {
        Ok(set) => Some(set),
        Err(e) => {
            issues.push(Issue {
                file: rel.into(),
                line: 1,
                reason: format!("не набор правил: {e}"),
            });
            None
        }
    }
}

/// Список строк из поля `key`; не-массив и не-строки — ошибки.
fn string_items<'a>(
    rel: &str,
    text: &str,
    field: Option<&'a Value>,
    key: &str,
    issues: &mut Vec<Issue>,
) -> Vec<&'a str> {
    let Some(field) = field else {
        return Vec::new();
    };
    let Some(items) = field.as_array() else {
        issues.push(Issue {
            file: rel.into(),
            line: line_of(text, &format!("\"{key}\""), 1),
            reason: format!("{key}: ожидался массив строк"),
        });
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        match item.as_str() {
            Some(s) => out.push(s),
            None => issues.push(Issue {
                file: rel.into(),
                line: line_of(text, &format!("\"{key}\""), 1),
                reason: format!("{key}: элемент {item} — не строка"),
            }),
        }
    }
    out
}

fn check_domains(rel: &str, text: &str, field: Option<&Value>, key: &str, issues: &mut Vec<Issue>) {
    let items = string_items(rel, text, field, key, issues);
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for item in items {
        let nth = seen.entry(item).or_insert(0);
        *nth += 1;
        let line = line_of(text, &quoted(item), *nth);
        if *nth > 1 {
            issues.push(Issue {
                file: rel.into(),
                line,
                reason: format!("{key}: повтор «{item}»"),
            });
            continue;
        }
        if let Err(e) = DomainPattern::parse(item) {
            issues.push(Issue {
                file: rel.into(),
                line,
                reason: format!("{key}: {e}"),
            });
        }
    }
}

fn check_subnets(rel: &str, text: &str, field: Option<&Value>, key: &str, issues: &mut Vec<Issue>) {
    let items = string_items(rel, text, field, key, issues);
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for item in items {
        let nth = seen.entry(item).or_insert(0);
        *nth += 1;
        let line = line_of(text, &quoted(item), *nth);
        if *nth > 1 {
            issues.push(Issue {
                file: rel.into(),
                line,
                reason: format!("{key}: повтор «{item}»"),
            });
            continue;
        }
        let reason = match item.trim().parse::<Ipv4Net>() {
            Ok(net) if net.trunc() != net => Some(format!(
                "{key}: «{item}» — биты хоста вне маски; имелось в виду {}?",
                net.trunc()
            )),
            Ok(_) => None,
            Err(_)
                if item.trim().parse::<Ipv6Net>().is_ok()
                    || item.trim().parse::<std::net::Ipv6Addr>().is_ok() =>
            {
                Some(format!(
                    "{key}: «{item}» — IPv6 в туннель не идёт (правило 9)"
                ))
            }
            Err(_) => match item.trim().parse::<std::net::Ipv4Addr>() {
                Ok(ip) => Some(format!("{key}: «{item}» — укажи маску: {ip}/32")),
                Err(_) => Some(format!("{key}: «{item}» — не подсеть IPv4 вида a.b.c.d/n")),
            },
        };
        if let Some(reason) = reason {
            issues.push(Issue {
                file: rel.into(),
                line,
                reason,
            });
        }
    }
}

fn check_modes_file(
    rel: &str,
    text: &str,
    set_ids: &BTreeSet<String>,
    issues: &mut Vec<Issue>,
) -> Vec<Mode> {
    let Some(value) = parse_json(rel, text, issues) else {
        return Vec::new();
    };
    let Some(items) = value.get("modes").and_then(Value::as_array) else {
        issues.push(Issue {
            file: rel.into(),
            line: 1,
            reason: "ожидался объект { \"modes\": [ … ] }".into(),
        });
        return Vec::new();
    };
    let before = issues.len();
    let mut ids: HashMap<String, usize> = HashMap::new();
    let mut defaults = 0;
    for item in items {
        let Some(obj) = item.as_object() else {
            issues.push(Issue {
                file: rel.into(),
                line: 1,
                reason: format!("режим {item} — не объект"),
            });
            continue;
        };
        let id = obj.get("id").and_then(Value::as_str).unwrap_or_default();
        let nth = ids.entry(id.to_owned()).or_insert(0);
        *nth += 1;
        let id_line = line_of(text, &format!("\"id\": {}", quoted(id)), *nth);
        if id.is_empty() {
            issues.push(Issue {
                file: rel.into(),
                line: id_line,
                reason: "режим без id".into(),
            });
        } else if *nth > 1 {
            issues.push(Issue {
                file: rel.into(),
                line: id_line,
                reason: format!("режим «{id}» объявлен повторно"),
            });
        }
        if obj
            .get("name")
            .and_then(Value::as_str)
            .is_none_or(|n| n.trim().is_empty())
        {
            issues.push(Issue {
                file: rel.into(),
                line: id_line,
                reason: format!("режим «{id}»: нет поля name"),
            });
        }
        if obj.get("default").and_then(Value::as_bool) == Some(true) {
            defaults += 1;
        }
        let sets = string_items(rel, text, obj.get("sets"), "sets", issues);
        let mut seen: HashMap<&str, usize> = HashMap::new();
        for set in sets {
            let n = seen.entry(set).or_insert(0);
            *n += 1;
            if !set_ids.contains(set) {
                issues.push(Issue {
                    file: rel.into(),
                    line: line_of(
                        text,
                        &quoted(set),
                        line_occurrence_after(text, id_line, set),
                    ),
                    reason: format!("режим «{id}»: набора «{set}» нет в state/rules/"),
                });
            }
        }
        check_domains(
            rel,
            text,
            obj.get("exclude_domains"),
            "exclude_domains",
            issues,
        );
        check_subnets(rel, text, obj.get("exclude_ips"), "exclude_ips", issues);
    }
    if defaults != 1 {
        issues.push(Issue {
            file: rel.into(),
            line: 1,
            reason: format!("default должен быть ровно у одного режима, сейчас у {defaults}"),
        });
    }
    if issues.len() != before {
        return Vec::new();
    }
    match serde_json::from_value::<braid_proto::ModesFile>(value) {
        Ok(file) => file.modes,
        Err(e) => {
            issues.push(Issue {
                file: rel.into(),
                line: 1,
                reason: format!("не файл режимов: {e}"),
            });
            Vec::new()
        }
    }
}

/// Порядковый номер первого вхождения `needle`, лежащего на строке `line` или ниже.
fn line_occurrence_after(text: &str, line: usize, needle: &str) -> usize {
    let needle = quoted(needle);
    let mut from = 0;
    let mut nth = 0;
    while let Some(pos) = text[from..].find(&needle) {
        let at = from + pos;
        nth += 1;
        if text[..at].matches('\n').count() + 1 >= line {
            return nth;
        }
        from = at + needle.len();
    }
    1
}
