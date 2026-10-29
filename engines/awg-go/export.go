// libawg: cgo c-shared над amneziawg-go/v3 (notes-awg.md §3, вариант a).
// Экспорт в C — только здесь; логика ядра — в instance.go, pkttun.go, bind.go.
//
// Правила границы: recover в каждом экспорте (паника Go = abort процесса
// вместе с UI); никаких signal.Notify (рантайм c-shared не трогает сигналы
// хозяина сверх своих); Go-указатели C не хранит — пакеты копируются в обе
// стороны; колбэки C зовутся через static inline трамплины.
package main

/*
#include <stdint.h>
#include <stdlib.h>
#define AWG_NO_PROTOTYPES
#include "awg.h"

static inline void awg_call_tx(awg_tx_fn f, void* ctx, const uint8_t* pkt, size_t len) { f(ctx, pkt, len); }
static inline int awg_call_protect(awg_protect_fn f, void* ctx, int fd) { return f(ctx, fd); }
static inline void awg_call_log(awg_log_fn f, void* ctx, int level, const char* msg) { f(ctx, level, msg); }
*/
import "C"

import (
	"errors"
	"fmt"
	"sync"
	"unsafe"

	"github.com/amnezia-vpn/amneziawg-go/v3/conn"
	"github.com/amnezia-vpn/amneziawg-go/v3/device"
)

// maxPacket — больше IP-пакета не бывает; защищает C.GoBytes от мусорной длины.
const maxPacket = 65535

// Таблица живых ядер: через границу C ходит int32-дескриптор, не указатель.
var handles = struct {
	sync.Mutex
	next int32
	m    map[int32]*instance
}{next: 1, m: map[int32]*instance{}}

func register(inst *instance) C.int32_t {
	handles.Lock()
	defer handles.Unlock()
	h := handles.next
	handles.next++
	handles.m[h] = inst
	return C.int32_t(h)
}

func lookup(h C.int32_t) *instance {
	handles.Lock()
	defer handles.Unlock()
	return handles.m[int32(h)]
}

func unregister(h C.int32_t) *instance {
	handles.Lock()
	defer handles.Unlock()
	inst := handles.m[int32(h)]
	delete(handles.m, int32(h))
	return inst
}

// goCallbacks оборачивает C-колбэки в Go-замыкания. ctx — указатель хозяина,
// Go его не разыменовывает и не хранит нигде, кроме замыканий.
func goCallbacks(cbs *C.awg_callbacks, ctx unsafe.Pointer) callbacks {
	var cb callbacks
	if cbs == nil {
		return cb
	}
	c := *cbs
	if c.tx != nil {
		tx := c.tx
		cb.tx = func(pkt []byte) {
			if len(pkt) == 0 {
				return
			}
			C.awg_call_tx(tx, ctx, (*C.uint8_t)(unsafe.Pointer(&pkt[0])), C.size_t(len(pkt)))
		}
	}
	if c.protect != nil {
		protect := c.protect
		cb.protect = func(fd int) bool {
			return C.awg_call_protect(protect, ctx, C.int(fd)) != 0
		}
	}
	if c.log != nil {
		logFn := c.log
		cb.log = func(level int, msg string) {
			cs := C.CString(msg)
			C.awg_call_log(logFn, ctx, C.int(level), cs)
			C.free(unsafe.Pointer(cs))
		}
	}
	return cb
}

// errorCode переводит ошибку Go в код для C: errno UAPI как есть, свои — AWG_ERR_*.
func errorCode(err error, fallback C.int) C.int {
	var ipcErr *device.IPCError
	switch {
	case errors.As(err, &ipcErr):
		return C.int(ipcErr.ErrorCode())
	case errors.Is(err, errProtect):
		return C.AWG_ERR_PROTECT
	case errors.Is(err, errQueueFull):
		return C.AWG_ERR_QUEUE_FULL
	case errors.Is(err, errClosed), errors.Is(err, errTunClosed):
		return C.AWG_ERR_CLOSED
	}
	return fallback
}

// guard — recover для экспорта: паника уходит в лог хозяина кодом AWG_ERR_PANIC.
func guard(cb callbacks, what string, ret *C.int) {
	if r := recover(); r != nil {
		cb.logf(logError, "паника в %s: %v", what, r)
		*ret = C.AWG_ERR_PANIC
	}
}

func (cb callbacks) logf(level int, format string, args ...any) {
	if cb.log != nil {
		cb.log(level, fmt.Sprintf(format, args...))
	}
}

//export awg_new
func awg_new(uapi *C.char, mtu C.int, cbs *C.awg_callbacks, ctx unsafe.Pointer) (ret C.int32_t) {
	cb := goCallbacks(cbs, ctx)
	defer func() {
		if r := recover(); r != nil {
			cb.logf(logError, "паника в awg_new: %v", r)
			ret = C.AWG_ERR_PANIC
		}
	}()
	if uapi == nil {
		return C.AWG_ERR_BAD_ARG
	}
	inst, err := newInstance(C.GoString(uapi), int(mtu), conn.NewDefaultBind(), cb)
	if err != nil {
		// Текст ошибки ядро уже отдало в log через Errorf; здесь — только код.
		return C.int32_t(errorCode(err, C.AWG_ERR_UP))
	}
	return register(inst)
}

//export awg_inject
func awg_inject(handle C.int32_t, pkt *C.uint8_t, n C.size_t) (ret C.int) {
	inst := lookup(handle)
	if inst == nil {
		return C.AWG_ERR_BAD_HANDLE
	}
	defer guard(inst.cb, "awg_inject", &ret)
	if pkt == nil || n == 0 || n > maxPacket {
		return C.AWG_ERR_BAD_ARG
	}
	if err := inst.inject(C.GoBytes(unsafe.Pointer(pkt), C.int(n))); err != nil {
		return errorCode(err, C.AWG_ERR_CLOSED)
	}
	return C.AWG_OK
}

//export awg_set_config
func awg_set_config(handle C.int32_t, uapi *C.char) (ret C.int) {
	inst := lookup(handle)
	if inst == nil {
		return C.AWG_ERR_BAD_HANDLE
	}
	defer guard(inst.cb, "awg_set_config", &ret)
	if uapi == nil {
		return C.AWG_ERR_BAD_ARG
	}
	if err := inst.setConfig(C.GoString(uapi)); err != nil {
		return errorCode(err, C.AWG_ERR_CLOSED)
	}
	return C.AWG_OK
}

//export awg_get_config
func awg_get_config(handle C.int32_t) (ret *C.char) {
	inst := lookup(handle)
	if inst == nil {
		return nil
	}
	defer func() {
		if r := recover(); r != nil {
			inst.cb.logf(logError, "паника в awg_get_config: %v", r)
			ret = nil
		}
	}()
	text, err := inst.getConfig()
	if err != nil {
		return nil
	}
	return C.CString(text)
}

//export awg_free_string
func awg_free_string(s *C.char) {
	defer func() { _ = recover() }()
	if s != nil {
		C.free(unsafe.Pointer(s))
	}
}

//export awg_bump_sockets
func awg_bump_sockets(handle C.int32_t) {
	inst := lookup(handle)
	if inst == nil {
		return
	}
	var ret C.int
	defer guard(inst.cb, "awg_bump_sockets", &ret)
	inst.bumpSockets()
}

//export awg_close
func awg_close(handle C.int32_t) {
	inst := unregister(handle)
	if inst == nil {
		return
	}
	var ret C.int
	defer guard(inst.cb, "awg_close", &ret)
	inst.close()
}

// main нужен buildmode=c-shared; как программа libawg не запускается.
func main() {}
