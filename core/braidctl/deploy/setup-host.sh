#!/usr/bin/env bash
# Установка на хост (Debian/Ubuntu, systemd): csqtt-server из апстрима amurcanov/csqtt
# на закреплённом коммите (shared/wire_protocol.rs = CSQTT-WIRE-3, ядро 2.1.9) и
# braid-subd из репозитория Braid. AWG не трогается (ист. 48).
#
# Запускает `braidctl host setup <id>` по SSH; руками:
#   sudo env BRAID_PUBLIC_IP=203.0.113.10 bash setup-host.sh install
#
# Идемпотентно: источники обновляются до пина, бинари собираются заново только
# если их нет, установщик апстрима (deploy.sh) запускается, только если бинарь
# csqtt на хосте отличается от собранного; install.sh braid-subd сам бережёт
# конфиг, ключ и токен. Повторный запуск без изменений ничего не перевыпускает.
#
# Переменные окружения (все необязательны, кроме BRAID_PUBLIC_IP):
#   BRAID_PUBLIC_IP          публичный IPv4 хоста — SAN сертификата braid-subd
#   BRAID_CSQTT_REPO         апстрим CSQTT (https://github.com/amurcanov/csqtt)
#   BRAID_CSQTT_COMMIT       пин апстрима
#   BRAID_CSQTT_PEER_PORT    UDP-порт сервера (46000), BRAID_CSQTT_WEB_PORT — панель (46002)
#   BRAID_SSH_PORT           порт SSH, который установщик апстрима держит открытым (22)
#   BRAID_SUBD_PORT          HTTPS-порт braid-subd (8443)
#   BRAID_REPO, BRAID_REF    репозиторий и ветка Braid для сборки braid-subd
#   BRAID_ENV_FILE           файл с CSQTT_WEB_USER, CSQTT_WEB_PASS, CSQTT_MAIN_PASSWORD (0600)
#   BRAID_CSQTT_BINARY       готовый linux-бинарь csqtt — вместо сборки
#   BRAID_SUBD_BINARY        готовый linux-бинарь braid-subd — вместо сборки
#   BRAID_SRC_DIR            куда клонировать исходники (/opt/braid/src)
#
#   setup-host.sh [--dry-run] [--help] install|status
set -euo pipefail

CSQTT_REPO="${BRAID_CSQTT_REPO:-https://github.com/amurcanov/csqtt}"
CSQTT_COMMIT="${BRAID_CSQTT_COMMIT:-b4ae5ce957b37d50414c91ae9afa96f2e98b68ac}"
CSQTT_WIRE="CSQTT-WIRE-3"
CSQTT_VERSION="2.1.9"
CSQTT_PEER_PORT="${BRAID_CSQTT_PEER_PORT:-46000}"
CSQTT_WEB_PORT="${BRAID_CSQTT_WEB_PORT:-46002}"
SSH_PORT="${BRAID_SSH_PORT:-22}"
SUBD_PORT="${BRAID_SUBD_PORT:-8443}"
PUBLIC_IP="${BRAID_PUBLIC_IP:-}"
BRAID_REPO="${BRAID_REPO:-https://github.com/alextixru/braid}"
BRAID_REF="${BRAID_REF:-main}"
ENV_FILE="${BRAID_ENV_FILE:-/root/.braid-setup/setup.env}"
CSQTT_BINARY="${BRAID_CSQTT_BINARY:-}"
SUBD_BINARY="${BRAID_SUBD_BINARY:-}"
SRC_DIR="${BRAID_SRC_DIR:-/opt/braid/src}"
CSQTT_SRC="$SRC_DIR/csqtt"
BRAID_SRC="$SRC_DIR/braid"
CSQTT_BIN=/usr/local/bin/csqtt
SUBD_BIN=/usr/local/bin/braid-subd
SUBD_CONF=/etc/braid/subd.toml
DRY_RUN=0
ACTION=""

usage() {
  cat <<'EOF'
Установка csqtt-server (апстрим, закреплённый коммит) и braid-subd на хост.

  sudo env BRAID_PUBLIC_IP=<ip> bash setup-host.sh [--dry-run] install
  bash setup-host.sh status

  --dry-run   напечатать план, ничего не делая
  --help      эта справка
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    install|status) ACTION="$1"; shift ;;
    *) echo "неизвестный аргумент: $1" >&2; usage >&2; exit 2 ;;
  esac
done
[[ -n "$ACTION" ]] || { usage >&2; exit 2; }

# В dry-run команды только печатаются с префиксом «+ ».
run() {
  if [[ $DRY_RUN -eq 1 ]]; then
    printf '+ %s\n' "$*"
  else
    "$@"
  fi
}
note() { printf '%s\n' "$*"; }
die() { printf 'BRAID_SETUP_ERROR|%s\n' "$*" >&2; exit 1; }

do_status() {
  note "csqtt: $(systemctl is-active csqtt 2>/dev/null || echo unknown)"
  [[ -x $CSQTT_BIN ]] && note "csqtt bin: $("$CSQTT_BIN" --version 2>/dev/null || echo '?')" || note "csqtt bin: нет"
  note "braid-subd: $(systemctl is-active braid-subd 2>/dev/null || echo unknown)"
  [[ -x $SUBD_BIN ]] && note "braid-subd bin: $("$SUBD_BIN" --version 2>/dev/null || echo '?')" || note "braid-subd bin: нет"
  [[ -f $SUBD_CONF ]] && note "braid-subd pin: $("$SUBD_BIN" --config "$SUBD_CONF" pin 2>/dev/null || echo '?')"
}

if [[ $ACTION == status ]]; then
  do_status
  exit 0
fi

[[ -n "$PUBLIC_IP" ]] || die "нужен BRAID_PUBLIC_IP — публичный IPv4 хоста"
if [[ $DRY_RUN -eq 0 && ${EUID:-$(id -u)} -ne 0 ]]; then
  die "нужен root: sudo $0"
fi

note "== 1. Пакеты для сборки"
if command -v apt-get >/dev/null 2>&1; then
  run env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
    git curl ca-certificates build-essential pkg-config
else
  note "apt-get не найден — поставь git, curl, gcc/g++ и pkg-config сам"
fi

note "== 2. Rust (rustup): нужен для сборки csqtt 2.1.9 (1.97.1 по rust-toolchain.toml) и braid-subd (stable)"
export CARGO_HOME="${CARGO_HOME:-/root/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-/root/.rustup}"
export PATH="$CARGO_HOME/bin:$PATH"
if command -v cargo >/dev/null 2>&1; then
  note "cargo уже есть — пропуск"
else
  run sh -c 'curl --proto =https --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable'
fi

note "== 3. Исходники csqtt-server: $CSQTT_REPO @ ${CSQTT_COMMIT:0:12}"
run install -d -m 0755 "$SRC_DIR"
if [[ -d "$CSQTT_SRC/.git" ]]; then
  run git -C "$CSQTT_SRC" fetch --quiet origin
else
  run git clone --quiet "$CSQTT_REPO" "$CSQTT_SRC"
fi
run git -C "$CSQTT_SRC" checkout --quiet "$CSQTT_COMMIT"
if [[ $DRY_RUN -eq 0 ]]; then
  grep -q "\"$CSQTT_WIRE\"" "$CSQTT_SRC/shared/wire_protocol.rs" \
    || die "апстрим на $CSQTT_COMMIT не $CSQTT_WIRE — пин не совместим с ядром клиента"
  grep -q "^version = \"$CSQTT_VERSION\"" "$CSQTT_SRC/rust-server/Cargo.toml" \
    || die "апстрим на $CSQTT_COMMIT не версии $CSQTT_VERSION"
else
  note "+ проверка: shared/wire_protocol.rs содержит $CSQTT_WIRE, rust-server 2.1.9"
fi

note "== 4. Бинарь csqtt"
if [[ -n "$CSQTT_BINARY" ]]; then
  note "готовый бинарь: $CSQTT_BINARY"
  NEW_CSQTT="$CSQTT_BINARY"
else
  # .cargo/config.toml апстрима целится в musl через zig; собираем под родной
  # gnu-триплет хоста (--target перекрывает конфиг), без zig и musl-tools.
  HOST_TRIPLE="$(rustc -vV 2>/dev/null | sed -n 's/^host: //p' || true)"
  HOST_TRIPLE="${HOST_TRIPLE:-x86_64-unknown-linux-gnu}"
  NEW_CSQTT="$CSQTT_SRC/rust-server/target/$HOST_TRIPLE/release/csqtt"
  if [[ -x "$NEW_CSQTT" ]]; then
    note "уже собран — пропуск"
  else
    run sh -c "cd '$CSQTT_SRC/rust-server' && cargo build --release --target '$HOST_TRIPLE'"
  fi
fi

note "== 5. csqtt-server: установщик апстрима (app/src/main/assets/deploy.sh install)"
same_csqtt=0
if [[ $DRY_RUN -eq 0 && -x "$CSQTT_BIN" && -f "$NEW_CSQTT" ]] && cmp -s "$CSQTT_BIN" "$NEW_CSQTT" \
   && [[ -f /etc/systemd/system/csqtt.service ]]; then
  same_csqtt=1
fi
if [[ $same_csqtt -eq 1 ]]; then
  note "уже установлен, бинарь совпадает — пропуск"
else
  if [[ $DRY_RUN -eq 0 ]]; then
    [[ -f "$ENV_FILE" ]] || die "нет $ENV_FILE с CSQTT_WEB_USER/CSQTT_WEB_PASS/CSQTT_MAIN_PASSWORD"
    [[ -f "$NEW_CSQTT" ]] || die "бинарь csqtt не собран: $NEW_CSQTT"
    # shellcheck disable=SC1090
    . "$ENV_FILE"
    : "${CSQTT_WEB_USER:?}" "${CSQTT_WEB_PASS:?}" "${CSQTT_MAIN_PASSWORD:?}"
    # Одноразовые файлы установщика (его контракт): бинарь, env панели,
    # deploy-overrides.json с основным паролем; device_id пустой — основной
    # пароль ни к какому устройству не привязан.
    (umask 077
      install -m 0755 "$NEW_CSQTT" /tmp/.csqtt-upload-server
      printf 'CSQTT_WEB_USER=%s\nCSQTT_WEB_PASS=%s\n' "$CSQTT_WEB_USER" "$CSQTT_WEB_PASS" > /tmp/.csqtt-upload-web.env
      printf '{"main_password": "%s", "device_id": ""}\n' "$CSQTT_MAIN_PASSWORD" > /tmp/.csqtt-upload-overrides.json)
  else
    note "+ (бинарь, CSQTT_WEB_USER/PASS и main_password из $ENV_FILE → /tmp/.csqtt-upload-*)"
  fi
  if [[ $DRY_RUN -eq 1 ]]; then
    run env CSQTT_PEER_PORT="$CSQTT_PEER_PORT" CSQTT_SSH_PORT="$SSH_PORT" CSQTT_WEB_PORT="$CSQTT_WEB_PORT" \
      CSQTT_DEPLOY_MODE=systemd bash "$CSQTT_SRC/app/src/main/assets/deploy.sh" install
  else
    # Полный вывод — в лог; на экран только маркеры и итоги установщика.
    env CSQTT_PEER_PORT="$CSQTT_PEER_PORT" CSQTT_SSH_PORT="$SSH_PORT" CSQTT_WEB_PORT="$CSQTT_WEB_PORT" \
      CSQTT_DEPLOY_MODE=systemd bash "$CSQTT_SRC/app/src/main/assets/deploy.sh" install 2>&1 \
      | tee /var/log/braid-csqtt-deploy.log | grep -E '^(CSQTT_|✓|\[✗\]|   )' || true
    grep -q '^CSQTT_DEPLOY_OK' /var/log/braid-csqtt-deploy.log \
      || die "deploy.sh апстрима не сообщил CSQTT_DEPLOY_OK — см. /var/log/braid-csqtt-deploy.log и /var/log/csqtt-install.log"
  fi
fi

note "== 6. Исходники Braid: $BRAID_REPO @ $BRAID_REF"
if [[ -n "$SUBD_BINARY" ]]; then
  note "готовый бинарь braid-subd: $SUBD_BINARY — исходники нужны только ради deploy/"
fi
if [[ -d "$BRAID_SRC/.git" ]]; then
  run git -C "$BRAID_SRC" fetch --quiet origin
else
  run git clone --quiet "$BRAID_REPO" "$BRAID_SRC"
fi
run git -C "$BRAID_SRC" checkout --quiet "$BRAID_REF"
run git -C "$BRAID_SRC" pull --quiet --ff-only origin "$BRAID_REF"

note "== 7. Бинарь braid-subd"
if [[ -n "$SUBD_BINARY" ]]; then
  NEW_SUBD="$SUBD_BINARY"
else
  NEW_SUBD="$BRAID_SRC/core/target/release/braid-subd"
  run sh -c "cd '$BRAID_SRC/core' && cargo build --release -p braid-subd"
fi

note "== 8. braid-subd: core/braid-subd/deploy/install.sh (идемпотентен: конфиг, ключ и токен бережёт)"
if [[ $DRY_RUN -eq 0 && ! -f "$SUBD_CONF" && "$SUBD_PORT" != 8443 ]]; then
  # Нестандартный порт задаётся до первой установки — потом install.sh конфиг не трогает.
  install -d -m 0750 /etc/braid
  sed "s/^listen = .*/listen = \"0.0.0.0:$SUBD_PORT\"/" "$BRAID_SRC/core/braid-subd/deploy/subd.toml" > "$SUBD_CONF"
  chmod 0640 "$SUBD_CONF"
fi
run bash "$BRAID_SRC/core/braid-subd/deploy/install.sh" --binary "$NEW_SUBD" --san "$PUBLIC_IP"

note "== 9. Порт braid-subd в файрволе"
if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q '^Status: active'; then
  run ufw allow "$SUBD_PORT/tcp"
else
  note "ufw не активен — порт $SUBD_PORT/tcp открывать не нужно (или открой сам)"
fi

note "== Готово"
if [[ $DRY_RUN -eq 0 ]]; then
  note "csqtt-server: $("$CSQTT_BIN" --version 2>/dev/null || echo '?'), UDP $CSQTT_PEER_PORT, панель $CSQTT_WEB_PORT"
  note "braid-subd: $("$SUBD_BIN" --version 2>/dev/null || echo '?'), порт $SUBD_PORT, пин $("$SUBD_BIN" --config "$SUBD_CONF" pin 2>/dev/null || echo '?')"
fi
echo "BRAID_SETUP_OK"
