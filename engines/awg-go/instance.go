package main

import (
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/amnezia-vpn/amneziawg-go/v3/conn"
	"github.com/amnezia-vpn/amneziawg-go/v3/device"
)

// Уровни log-колбэка (совпадают с AWG_LOG_* в awg.h).
const (
	logError   = 1
	logVerbose = 2
)

const (
	injectQueue  = 512 // пакетов роутера в очереди на шифрование
	bumpAttempts = 10  // попыток переоткрыть сокеты при смене сети (как wgBumpSockets Amnezia)
	bumpPause    = 500 * time.Millisecond
)

var errClosed = errors.New("libawg: ядро закрыто")

// callbacks — то, что движок просит у хозяина: отдать пакет, защитить сокет, записать лог.
type callbacks struct {
	tx      func(pkt []byte)            // расшифрованный IP-пакет от сервера; буфер ядра, копировать
	protect func(fd int) bool           // Android: VpnService.protect; nil — не нужно
	log     func(level int, msg string) // nil — молчать
}

// instance — одно ядро AWG: device.Device поверх pktTun и protectBind.
//
// Колбэки хозяина проходят через enter/leave: после close ни один из них
// не вызывается, а close дожидается уже начатых — хозяин может освобождать
// свой контекст сразу после возврата close. Внутри колбэков замки не держатся,
// поэтому tx → inject (обратный вызов из роутера) не блокируется.
type instance struct {
	dev *device.Device
	tun *pktTun
	cb  callbacks

	gate     sync.Mutex // защищает closed и вход в inflight
	closed   bool
	inflight sync.WaitGroup
}

// newInstance собирает ядро: NewDevice → IpcSet(uapi) → явный Up()
// (EventUp от tun не посылается, иначе Up случился бы до конфига).
func newInstance(uapi string, mtu int, bind conn.Bind, cb callbacks) (*instance, error) {
	if mtu <= 0 {
		mtu = device.DefaultMTU
	}
	inst := &instance{cb: cb}
	inst.tun = newPktTun(mtu, injectQueue, inst.deliver)
	logger := &device.Logger{
		Verbosef: func(format string, args ...any) { inst.logf(logVerbose, format, args...) },
		Errorf:   func(format string, args ...any) { inst.logf(logError, format, args...) },
	}
	inst.dev = device.NewDevice(inst.tun, &protectBind{Bind: bind, protect: inst.protectFd}, logger)
	if err := inst.dev.IpcSet(uapi); err != nil {
		inst.close()
		return nil, err
	}
	inst.tun.resync() // ядро уже ждёт в Read со смещением для S4 = 0
	if err := inst.dev.Up(); err != nil {
		inst.close()
		return nil, fmt.Errorf("up: %w", err)
	}
	return inst, nil
}

// enter открывает вход в ядро или колбэк; false — ядро закрыто.
func (i *instance) enter() bool {
	i.gate.Lock()
	defer i.gate.Unlock()
	if i.closed {
		return false
	}
	i.inflight.Add(1)
	return true
}

func (i *instance) leave() { i.inflight.Done() }

func (i *instance) deliver(pkt []byte) {
	if !i.enter() {
		return
	}
	defer i.leave()
	if i.cb.tx != nil {
		i.cb.tx(pkt)
	}
}

func (i *instance) protectFd(fd int) bool {
	if i.cb.protect == nil {
		return true
	}
	if !i.enter() {
		return false
	}
	defer i.leave()
	return i.cb.protect(fd)
}

func (i *instance) logf(level int, format string, args ...any) {
	if i.cb.log == nil {
		return
	}
	if !i.enter() {
		return
	}
	defer i.leave()
	i.cb.log(level, fmt.Sprintf(format, args...))
}

// inject — IP-пакет от роутера в сторону сервера. Буфер переходит ядру.
func (i *instance) inject(pkt []byte) error {
	if !i.enter() {
		return errClosed
	}
	defer i.leave()
	return i.tun.inject(pkt)
}

// setConfig применяет дополнительный UAPI-текст (например, новый endpoint).
func (i *instance) setConfig(uapi string) error {
	if !i.enter() {
		return errClosed
	}
	defer i.leave()
	if err := i.dev.IpcSet(uapi); err != nil {
		return err
	}
	i.tun.resync() // конфиг мог сменить S4
	return nil
}

// getConfig — текст IpcGet: содержит ключи, хозяин его не логирует.
func (i *instance) getConfig() (string, error) {
	if !i.enter() {
		return "", errClosed
	}
	defer i.leave()
	return i.dev.IpcGet()
}

// bumpSockets переоткрывает UDP-сокеты после смены сети и шлёт keepalive
// на текущих ключах — в фоне, чтобы не держать вызывающий поток до 5 с.
func (i *instance) bumpSockets() {
	if !i.enter() {
		return
	}
	go func() {
		defer i.leave()
		defer func() {
			if r := recover(); r != nil {
				i.logf(logError, "паника в bumpSockets: %v", r)
			}
		}()
		for n := 0; n < bumpAttempts; n++ {
			err := i.dev.BindUpdate()
			if err == nil {
				i.dev.SendKeepalivesToPeersWithCurrentKeypair()
				return
			}
			i.logf(logError, "переоткрытие сокетов: %v", err)
			time.Sleep(bumpPause)
		}
	}()
}

// close останавливает ядро, закрывает tun и сокеты; повторный вызов безвреден.
// Нельзя звать из колбэков ядра — close ждёт их завершения.
func (i *instance) close() {
	i.gate.Lock()
	if i.closed {
		i.gate.Unlock()
		return
	}
	i.closed = true
	i.gate.Unlock()
	i.dev.Close()
	i.inflight.Wait()
}
