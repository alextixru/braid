package main

import (
	"errors"
	"os"
	"sync"

	"github.com/amnezia-vpn/amneziawg-go/v3/tun"
)

var (
	errTunClosed = errors.New("libawg: tun закрыт")
	errQueueFull = errors.New("libawg: очередь на отправку полна")
)

// pktTun — tun.Device поверх канала, без настоящего интерфейса (ист. 18):
// роутер кладёт IP-пакеты через inject, ядро AWG забирает их в Read;
// расшифрованные пакеты ядро отдаёт в Write, а мы — в колбэк deliver.
//
// Ядро читает со смещением offset = 16 + S4 (меняется между вызовами) и
// пишет со смещением 16; область до offset — его служебная, мы её не трогаем.
// BatchSize = 1: по пакету на вызов, как tuntest.ChannelTUN.
// EventUp не посылается — Up() вызывает владелец после IpcSet.
type pktTun struct {
	mtu     int
	in      chan []byte      // роутер → ядро AWG (inject → Read)
	deliver func(pkt []byte) // ядро AWG → роутер (Write); вызывается синхронно, буфер ядра
	events  chan tun.Event
	kick    chan struct{} // resync: разбудить Read без пакета
	done    chan struct{}
	once    sync.Once
}

func newPktTun(mtu int, queue int, deliver func(pkt []byte)) *pktTun {
	return &pktTun{
		mtu:     mtu,
		in:      make(chan []byte, queue),
		deliver: deliver,
		events:  make(chan tun.Event, 1),
		kick:    make(chan struct{}, 1),
		done:    make(chan struct{}),
	}
}

// resync будит заблокированный Read без пакета (0 пакетов, nil): ядро
// заново читает S4 и зовёт Read с актуальным смещением. Нужно после IpcSet —
// RoutineReadFromTUN берёт смещение до блокировки в Read, и первый пакет после
// конфигурации иначе ушёл бы с нулевым S4 (так теряется первый пакет и в
// собственном тесте amneziawg-go).
func (t *pktTun) resync() {
	select {
	case t.kick <- struct{}{}:
	default:
	}
}

// inject ставит пакет в очередь на шифрование. Буфер переходит во владение tun.
// Полная очередь — пакет отбрасывается (как у перегруженного интерфейса).
func (t *pktTun) inject(pkt []byte) error {
	select {
	case <-t.done:
		return errTunClosed
	default:
	}
	select {
	case t.in <- pkt:
		return nil
	case <-t.done:
		return errTunClosed
	default:
		return errQueueFull
	}
}

func (t *pktTun) File() *os.File           { return nil }
func (t *pktTun) MTU() (int, error)        { return t.mtu, nil }
func (t *pktTun) Name() (string, error)    { return "braid-awg", nil }
func (t *pktTun) Events() <-chan tun.Event { return t.events }
func (t *pktTun) BatchSize() int           { return 1 }

// Read ждёт один пакет и кладёт его в bufs[0][offset:]. После Close — ошибка
// (штатно: ядро само завершает RoutineReadFromTUN).
func (t *pktTun) Read(bufs [][]byte, sizes []int, offset int) (int, error) {
	if len(bufs) == 0 || len(sizes) == 0 {
		return 0, nil
	}
	select {
	case <-t.kick:
		return 0, nil // смещение могло смениться — пусть ядро перечитает S4
	default:
	}
	select {
	case <-t.done:
		return 0, os.ErrClosed
	case <-t.kick:
		return 0, nil
	case pkt := <-t.in:
		if offset < 0 || offset+len(pkt) > len(bufs[0]) {
			// Не влезает в буфер ядра — отбрасываем, как слишком большой пакет.
			sizes[0] = 0
			return 1, nil
		}
		sizes[0] = copy(bufs[0][offset:], pkt)
		return 1, nil
	}
}

// Write отдаёт каждый пакет без служебного заголовка в deliver. Буферы
// принадлежат ядру и переиспользуются после возврата — получатель копирует.
func (t *pktTun) Write(bufs [][]byte, offset int) (int, error) {
	for i, b := range bufs {
		select {
		case <-t.done:
			return i, os.ErrClosed
		default:
		}
		if offset < 0 || offset >= len(b) {
			continue
		}
		t.deliver(b[offset:])
	}
	return len(bufs), nil
}

// Close будит Read и закрывает канал событий; повторный вызов безвреден.
func (t *pktTun) Close() error {
	t.once.Do(func() {
		close(t.done)
		close(t.events)
	})
	return nil
}
