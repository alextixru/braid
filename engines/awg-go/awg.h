/* libawg — AmneziaWG (amneziawg-go/v3) как библиотека для роутера Braid.
 *
 * Ядро не создаёт интерфейсов и не трогает маршруты и DNS: IP-пакеты
 * приходят через awg_inject, расшифрованные пакеты от сервера уходят в
 * колбэк tx. Конфиг — текст UAPI WireGuard (key=value\n; hex-ключи,
 * endpoint IP-литералом, bool 1/0) с полями AmneziaWG (jc/jmin/jmax,
 * s1-s4, h1-h4, i1-i5, поля 3.1).
 *
 * Потоки: все функции потокобезопасны. Колбэки вызываются на потоках
 * Go-рантайма: tx — синхронно, буфер принадлежит ядру и переиспользуется
 * после возврата (получатель копирует); protect — из открытия сокетов
 * (awg_new, awg_bump_sockets); log — откуда угодно. После возврата
 * awg_close колбэки больше не вызываются — ctx можно освобождать.
 * awg_close нельзя звать из колбэков. Паника внутри экспорта
 * перехватывается и возвращается кодом AWG_ERR_PANIC.
 */
#ifndef BRAID_AWG_H
#define BRAID_AWG_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Расшифрованный IP-пакет от сервера → роутер. */
typedef void (*awg_tx_fn)(void* ctx, const uint8_t* pkt, size_t len);
/* Android: VpnService.protect(fd); вернуть 1 — защищён, 0 — отказ (сокет закрывается, старт не удаётся). */
typedef int (*awg_protect_fn)(void* ctx, int fd);
/* Строка журнала ядра; level — AWG_LOG_*. Секретов в сообщениях нет, конфиг и IpcGet не логируются. */
typedef void (*awg_log_fn)(void* ctx, int level, const char* msg);

typedef struct awg_callbacks {
    awg_tx_fn      tx;      /* обязателен */
    awg_protect_fn protect; /* NULL — защита не нужна (macOS, Linux) */
    awg_log_fn     log;     /* NULL — молчать */
} awg_callbacks;

enum {
    AWG_LOG_ERROR   = 1,
    AWG_LOG_VERBOSE = 2
};

/* Коды возврата. Ошибки конфига (awg_new, awg_set_config) — отрицательный
 * errno из UAPI amneziawg-go: -EINVAL (-22) кривое значение, -EPROTO (-71)
 * кривая строка, -EADDRINUSE (-98) порт занят, -EIO (-5). Свои коды — ниже -1000,
 * чтобы не пересекаться с errno. */
enum {
    AWG_OK             = 0,
    AWG_ERR_BAD_HANDLE = -1000, /* нет такого ядра (уже закрыто) */
    AWG_ERR_PANIC      = -1001, /* паника внутри Go, перехвачена */
    AWG_ERR_CLOSED     = -1002, /* ядро закрывается */
    AWG_ERR_QUEUE_FULL = -1003, /* очередь на шифрование полна, пакет отброшен */
    AWG_ERR_BAD_ARG    = -1004, /* NULL или пустой/слишком большой пакет */
    AWG_ERR_UP         = -1005, /* сокеты не открылись (Up) */
    AWG_ERR_PROTECT    = -1006  /* колбэк protect отказал */
};

#ifndef AWG_NO_PROTOTYPES
/* Создать ядро: NewDevice → IpcSet(uapi) → Up(). Возвращает дескриптор ≥ 1
 * или код ошибки < 0. mtu — MTU интерфейса роутера (≤ 0 — 1420). cbs копируется. */
int32_t awg_new(const char* uapi, int mtu, const awg_callbacks* cbs, void* ctx);
/* IP-пакет в сторону сервера (src уже адрес движка в туннеле). Копируется. */
int awg_inject(int32_t handle, const uint8_t* pkt, size_t len);
/* Дополнительный UAPI-текст поверх текущего (например, новый endpoint). */
int awg_set_config(int32_t handle, const char* uapi);
/* Текст IpcGet (last_handshake_time_sec, rx_bytes, tx_bytes, listen_port, …);
 * содержит ключи — не логировать. NULL — ядра нет. Освобождать awg_free_string. */
char* awg_get_config(int32_t handle);
void awg_free_string(char* s);
/* Смена сети: переоткрыть сокеты (до 10 попыток с паузой 0,5 с) и послать
 * keepalive на текущих ключах. Возвращается сразу, работает в фоне. */
void awg_bump_sockets(int32_t handle);
/* Остановить ядро, закрыть tun и сокеты, дождаться колбэков. Повторно — безвредно. */
void awg_close(int32_t handle);
#endif

#ifdef __cplusplus
}
#endif

#endif /* BRAID_AWG_H */
