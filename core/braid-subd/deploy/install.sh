#!/usr/bin/env bash
# Установка braid-subd на Debian/Ubuntu с systemd.
#
# Идемпотентно: существующие конфиг, админский токен, ключ с сертификатом и
# файлы подписок не трогаются; бинарь и юнит обновляются; сервис перезапускается.
# Повторный запуск без изменений ничего не ломает (ист. 48).
#
#   sudo ./install.sh [--binary PATH] [--san IP]... [--dry-run]
#
#   --binary PATH  откуда брать braid-subd (по умолчанию рядом со скриптом
#                  или ../../target/release/braid-subd)
#   --san IP       IP или имя хоста для сертификата; по умолчанию первый адрес
#                  хоста (на пин SPKI не влияет)
#   --dry-run      напечатать план, ничего не делая
set -euo pipefail

SERVICE_USER=braid-subd
CONF_DIR=/etc/braid
DATA_DIR=/var/lib/braid-subd
BIN=/usr/local/bin/braid-subd
UNIT=/etc/systemd/system/braid-subd.service
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

BINARY=""
SANS=()
DRY_RUN=0

usage() {
  cat <<'EOF'
Установка braid-subd (Debian/Ubuntu, systemd).

  sudo ./install.sh [--binary PATH] [--san IP]... [--dry-run]

  --binary PATH  откуда брать braid-subd (по умолчанию рядом со скриптом
                 или ../../target/release/braid-subd)
  --san IP       IP или имя хоста для сертификата (можно несколько)
  --dry-run      напечатать план, ничего не делая
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --binary) BINARY="${2:?--binary требует путь}"; shift 2 ;;
    --san) SANS+=("${2:?--san требует адрес}"); shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "неизвестный аргумент: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# В dry-run команды только печатаются с префиксом «+ ».
run() {
  if [[ $DRY_RUN -eq 1 ]]; then
    printf '+ %s\n' "$*"
  else
    "$@"
  fi
}
note() { printf '%s\n' "$*"; }

if [[ -z "$BINARY" ]]; then
  for candidate in "$SCRIPT_DIR/braid-subd" "$SCRIPT_DIR/../../target/release/braid-subd"; do
    if [[ -x "$candidate" ]]; then BINARY="$candidate"; break; fi
  done
fi
if [[ -z "$BINARY" || ! -f "$BINARY" ]]; then
  if [[ $DRY_RUN -eq 1 ]]; then
    note "бинарь не найден (${BINARY:-кандидатов нет}) — в dry-run продолжаем"
    BINARY="${BINARY:-braid-subd}"
  else
    echo "бинарь braid-subd не найден: собери \`make subd\` или укажи --binary" >&2
    exit 1
  fi
fi

if [[ $DRY_RUN -eq 0 && ${EUID:-$(id -u)} -ne 0 ]]; then
  echo "нужен root: sudo $0" >&2
  exit 1
fi

# SAN по умолчанию — первый адрес хоста (hostname -I есть только на Linux).
if [[ ${#SANS[@]} -eq 0 ]] && hostname -I >/dev/null 2>&1; then
  first_ip="$(hostname -I | awk '{print $1}')"
  if [[ -n "$first_ip" ]]; then SANS+=("$first_ip"); fi
fi
san_args=()
for s in ${SANS[@]+"${SANS[@]}"}; do san_args+=(--san "$s"); done

note "== 1. Пользователь $SERVICE_USER"
if id -u "$SERVICE_USER" >/dev/null 2>&1; then
  note "уже есть — пропуск"
else
  run useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin --user-group "$SERVICE_USER"
fi

note "== 2. Каталоги"
run install -d -m 0750 -o "$SERVICE_USER" -g "$SERVICE_USER" "$DATA_DIR" "$DATA_DIR/sub" "$DATA_DIR/telemetry"
run install -d -m 0750 -o root -g "$SERVICE_USER" "$CONF_DIR"

note "== 3. Бинарь $BIN"
run install -m 0755 "$BINARY" "$BIN"

note "== 4. Конфиг $CONF_DIR/subd.toml"
if [[ -f "$CONF_DIR/subd.toml" ]]; then
  note "уже есть — оставлен"
else
  run install -m 0640 -o root -g "$SERVICE_USER" "$SCRIPT_DIR/subd.toml" "$CONF_DIR/subd.toml"
fi

note "== 5. Админский токен $CONF_DIR/admin_token"
if [[ -f "$CONF_DIR/admin_token" ]]; then
  note "уже есть — оставлен"
else
  if [[ $DRY_RUN -eq 1 ]]; then
    note "+ (32 случайных байта из /dev/urandom в base64url) > $CONF_DIR/admin_token"
  else
    (umask 077; head -c 32 /dev/urandom | base64 | tr '+/' '-_' | tr -d '=\n' > "$CONF_DIR/admin_token"; echo >> "$CONF_DIR/admin_token")
  fi
  run chown root:"$SERVICE_USER" "$CONF_DIR/admin_token"
  run chmod 0640 "$CONF_DIR/admin_token"
fi

note "== 6. Сертификат (gen-cert существующий ключ не трогает, печатает пин)"
run "$BIN" --config "$CONF_DIR/subd.toml" gen-cert ${san_args[@]+"${san_args[@]}"}
run chown root:"$SERVICE_USER" "$CONF_DIR/subd.key" "$CONF_DIR/subd.crt"
run chmod 0640 "$CONF_DIR/subd.key"
run chmod 0644 "$CONF_DIR/subd.crt"

note "== 7. Юнит systemd"
run install -m 0644 "$SCRIPT_DIR/braid-subd.service" "$UNIT"
run systemctl daemon-reload
run systemctl enable braid-subd
run systemctl restart braid-subd

note "Готово. Пин для ссылок: $BIN --config $CONF_DIR/subd.toml pin"
