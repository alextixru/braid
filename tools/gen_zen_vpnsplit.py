#!/usr/bin/env python3
"""Сборка split-туннеля для сервера zen из списков AmneziaVPN в этой папке.

На выходе два файла, которые кладутся на zen:
  vpnsplit.nft  -> /etc/vpnsplit/vpnsplit.nft      (IPv4-подсети в set via_vpn4)
  vpnsplit.conf -> /etc/dnsmasq.d/vpnsplit.conf    (домены: dnsmasq кладёт их IP в тот же set)

Берутся все наборы правил Braid state/rules/*.json (поля domains и ips;
исключения exclude_* здесь не учитываются). Наборы только IPv4: на zen туннель
только IPv4, dnsmasq режет AAAA. Wildcard «*.x» сводится к «x»: dnsmasq и так
ловит поддомены.

EXCLUDE — то, что на сервере должно идти мимо туннеля: zen сам живёт на Beget,
а GitHub с него и так быстрый (медленный он только у провайдера мака).

Запуск: python3 tools/gen_zen_vpnsplit.py <папка_вывода> [endpoint-IP ...]
Endpoint VPN-сервера обязательно передать: если он попадёт в set, туннель
завернёт сам в себя. Дополнительные подсети «никогда в туннель» — в
state/never.txt (по CIDR на строку, # — комментарий; файл необязателен).
"""
import ipaddress
import json
import re
import sys
from pathlib import Path

RULES = Path(__file__).resolve().parent.parent / "state" / "rules"

# Точные имена: отдельные заблокированные страницы вроде x.github.io остаются в туннеле
EXCLUDE = {
    "beget.com",
    "github.com", "github.io", "githubassets.com", "githubusercontent.com", "ghcr.io",
    "codeload.github.com", "pkg.github.com",
    "objects.githubusercontent.com", "raw.githubusercontent.com",
}

# Эти адреса никогда не должны уходить в туннель: только общеизвестные диапазоны.
# Свои подсети (сеть хоста и т. п.) — в state/never.txt (вне git) и в аргументах.
NEVER = ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "100.64.0.0/10",
         "127.0.0.0/8", "198.18.0.0/15"]
NEVER_FILE = RULES.parent / "never.txt"

NFT_HEAD = """#!/usr/sbin/nft -f
add table inet vpnsplit
delete table inet vpnsplit
table inet vpnsplit {
  set via_vpn4 { type ipv4_addr; flags interval; auto-merge;
    elements = {
"""
NFT_TAIL = """  } }
  chain output {
    type route hook output priority mangle; policy accept;
    ct mark 0x64 meta mark set 0x64 counter return
    ct state new ip daddr @via_vpn4 counter meta mark set 0x64 ct mark set 0x64
  }
  chain postrouting {
    type nat hook postrouting priority srcnat; policy accept;
    oifname "awg0" counter masquerade
  }
  # MSS под MTU туннеля: сокет открывается по маршруту eth0 (1500), а уходит
  # в awg0 (1420). Без этого первый полный сегмент теряется, TLS ждёт ~5 с
  chain mss {
    type filter hook postrouting priority mangle; policy accept;
    oifname "awg0" tcp flags syn tcp option maxseg size set rt mtu
  }
}
"""
DNS_HEAD = """# split-tunnel: домены -> nftables set via_vpn4, резолв через туннель
# собрано tools/gen_zen_vpnsplit.py из state/rules/*.json
listen-address=127.0.0.1,172.17.0.1
bind-interfaces
no-resolv
no-hosts
cache-size=10000
filter-AAAA
server=198.18.18.18
"""
# dnsmasq не читает строки длиннее ~1024 символов
MAX_LINE = 900


def excluded(d):
    return d in EXCLUDE


def never_networks(extra):
    """NEVER + подсети из state/never.txt + переданные (endpoint-IP)."""
    nets = [ipaddress.ip_network(n) for n in NEVER]
    if NEVER_FILE.exists():
        for line in NEVER_FILE.read_text().splitlines():
            cidr = line.split("#", 1)[0].strip()
            if cidr:
                nets.append(ipaddress.ip_network(cidr, strict=False))
    return nets + list(extra)


def main():
    out = Path(sys.argv[1])
    endpoints = [ipaddress.ip_network(a) for a in sys.argv[2:]]
    if not endpoints:
        sys.exit("нужен хотя бы один endpoint-IP VPN-сервера")
    never = never_networks(endpoints)

    domains, nets = set(), []
    for f in sorted(RULES.glob("*.json")):
        rules = json.loads(f.read_text())
        for h in rules.get("ips", []):
            nets.append(ipaddress.ip_network(h.strip(), strict=False))
        for h in rules.get("domains", []):
            d = re.sub(r"^(\*\.)+", "", h.strip()).lower().rstrip(".")
            if d and not excluded(d):
                domains.add(d)

    nets = list(ipaddress.collapse_addresses(nets))
    bad = [(n, b) for n in nets for b in never if n.overlaps(b)]
    if bad:
        sys.exit("в set попали запрещённые адреса: %s" % bad[:10])

    # домены, уже покрытые родителем, не дублируем
    keep = sorted(d for d in domains
                  if not any(".".join(d.split(".")[i:]) in domains
                             for i in range(1, len(d.split(".")) - 1)))

    out.mkdir(parents=True, exist_ok=True)
    (out / "vpnsplit.nft").write_text(
        NFT_HEAD + "".join(f"      {n},\n" for n in nets) + NFT_TAIL)
    lines = [DNS_HEAD]
    chunk = []
    for d in keep + [None]:
        if d is None or len("/".join(chunk + [d])) > MAX_LINE - 40:
            part = "/".join(chunk)
            lines.append(f"server=/{part}/1.1.1.1\n")
            lines.append(f"nftset=/{part}/inet#vpnsplit#via_vpn4\n")
            chunk = []
        if d is not None:
            chunk.append(d)
    (out / "vpnsplit.conf").write_text("".join(lines))
    print(f"домены {len(keep)}, подсети {len(nets)} -> {out}")


if __name__ == "__main__":
    main()
