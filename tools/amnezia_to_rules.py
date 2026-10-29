#!/usr/bin/env python3
"""Перенос списка AmneziaVPN в набор правил Braid.

Вход — amnezia_*.json: [{"hostname": "...", "ip": "..."}]. Выход — RuleSet
(state/rules/<id>.json): имена в `domains` как есть (с `*.x`, включая
многоуровневые `*.*.x`), IPv4-адреса и подсети в `ips` (адрес без маски → /32).
IPv6 в туннель не идёт (правило: IPv4-only) — такие записи пропускаются с
предупреждением. Порядок записей сохраняется, точные дубликаты убираются.

Запуск: python3 tools/amnezia_to_rules.py <amnezia.json> <id> <Название> [<выход.json>]
По умолчанию выход — state/rules/<id>.json рядом с репозиторием.
"""
import ipaddress
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
IP_RE = re.compile(r"^\d{1,3}(\.\d{1,3}){3}(/\d{1,2})?$")


def convert(entries):
    domains, ips, skipped = [], [], []
    seen = set()
    for item in entries:
        host = (item.get("hostname") or "").strip()
        value = host or (item.get("ip") or "").strip()
        if not value or value in seen:
            continue
        seen.add(value)
        if ":" in value:
            skipped.append(value)
            continue
        if IP_RE.match(value):
            ipaddress.ip_network(value, strict=False)  # проверка синтаксиса
            ips.append(value if "/" in value else f"{value}/32")
        else:
            domains.append(value.lower().rstrip("."))
    return domains, ips, skipped


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    src, set_id, name = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
    out = Path(sys.argv[4]) if len(sys.argv) > 4 else ROOT / "state" / "rules" / f"{set_id}.json"
    domains, ips, skipped = convert(json.loads(src.read_text()))
    rule_set = {
        "id": set_id,
        "name": name,
        "domains": domains,
        "ips": ips,
        "exclude_domains": [],
        "exclude_ips": [],
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(rule_set, indent=2, ensure_ascii=False) + "\n")
    print(f"{out}: домены {len(domains)}, подсети {len(ips)}" + (f", пропущено IPv6 {len(skipped)}" if skipped else ""))


if __name__ == "__main__":
    main()
