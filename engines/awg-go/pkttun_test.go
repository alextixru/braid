package main

import (
	"bytes"
	"errors"
	"testing"
	"time"

	"github.com/amnezia-vpn/amneziawg-go/v3/device"
	"github.com/amnezia-vpn/amneziawg-go/v3/tun"
)

// Шов «свой tun.Device» (interfaces.md): ядро AWG читает пакет со смещением
// offset = 16 + S4, которое меняется между вызовами; пишет — со смещением 16.

var _ tun.Device = (*pktTun)(nil)

func samplePacket(n int) []byte {
	pkt := make([]byte, n)
	for i := range pkt {
		pkt[i] = byte(i*7 + 3)
	}
	pkt[0] = 0x45 // IPv4, IHL=5
	return pkt
}

// Read кладёт пакет ровно по offset = 16 + S4 и не трогает область до него.
func TestPktTunReadAtTransportOffset(t *testing.T) {
	const s4 = 25
	offset := device.MessageTransportHeaderSize + s4
	tn := newPktTun(1420, 8, func([]byte) {})
	defer tn.Close()

	pkt := samplePacket(60)
	if err := tn.inject(pkt); err != nil {
		t.Fatalf("inject: %v", err)
	}

	buf := make([]byte, 200)
	for i := range buf {
		buf[i] = 0xee
	}
	bufs := [][]byte{buf}
	sizes := make([]int, 1)
	n, err := tn.Read(bufs, sizes, offset)
	if err != nil || n != 1 {
		t.Fatalf("Read: n=%d err=%v, ожидалось 1 пакет", n, err)
	}
	if sizes[0] != len(pkt) {
		t.Fatalf("sizes[0]=%d, ожидалось %d", sizes[0], len(pkt))
	}
	if !bytes.Equal(buf[offset:offset+sizes[0]], pkt) {
		t.Fatalf("пакет лежит не по смещению %d", offset)
	}
	for i := 0; i < offset; i++ {
		if buf[i] != 0xee {
			t.Fatalf("область до offset затёрта в байте %d", i)
		}
	}
}

// Смещение переменное: второй вызов с другим S4 кладёт пакет по новому смещению.
func TestPktTunReadOffsetVaries(t *testing.T) {
	tn := newPktTun(1420, 8, func([]byte) {})
	defer tn.Close()
	pkt := samplePacket(40)
	for _, s4 := range []int{0, 12, 100} {
		offset := device.MessageTransportHeaderSize + s4
		if err := tn.inject(pkt); err != nil {
			t.Fatalf("inject: %v", err)
		}
		buf := make([]byte, 512)
		sizes := make([]int, 1)
		n, err := tn.Read([][]byte{buf}, sizes, offset)
		if err != nil || n != 1 || sizes[0] != len(pkt) {
			t.Fatalf("S4=%d: n=%d sizes=%v err=%v", s4, n, sizes, err)
		}
		if !bytes.Equal(buf[offset:offset+len(pkt)], pkt) {
			t.Fatalf("S4=%d: пакет не по смещению %d", s4, offset)
		}
	}
}

// Write отдаёт в deliver ровно пакет без заголовка (offset = 16).
func TestPktTunWriteStripsOffset(t *testing.T) {
	var got [][]byte
	tn := newPktTun(1420, 8, func(p []byte) { got = append(got, append([]byte(nil), p...)) })
	defer tn.Close()

	pkt := samplePacket(50)
	buf := make([]byte, device.MessageTransportOffsetContent+len(pkt))
	copy(buf[device.MessageTransportOffsetContent:], pkt)
	n, err := tn.Write([][]byte{buf}, device.MessageTransportOffsetContent)
	if err != nil || n != 1 {
		t.Fatalf("Write: n=%d err=%v", n, err)
	}
	if len(got) != 1 || !bytes.Equal(got[0], pkt) {
		t.Fatalf("deliver получил %v, ожидался пакет длиной %d", got, len(pkt))
	}
}

// Close будит заблокированный Read — иначе Device.Close() повиснет.
func TestPktTunCloseWakesRead(t *testing.T) {
	tn := newPktTun(1420, 8, func([]byte) {})
	done := make(chan error, 1)
	go func() {
		buf := make([]byte, 100)
		_, err := tn.Read([][]byte{buf}, make([]int, 1), 16)
		done <- err
	}()
	time.Sleep(20 * time.Millisecond)
	if err := tn.Close(); err != nil {
		t.Fatalf("Close: %v", err)
	}
	select {
	case err := <-done:
		if err == nil {
			t.Fatalf("Read после Close должен вернуть ошибку")
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("Read не проснулся после Close")
	}
	// Повторный Close безвреден, inject после Close — ошибка.
	if err := tn.Close(); err != nil {
		t.Fatalf("второй Close: %v", err)
	}
	if err := tn.inject(samplePacket(30)); !errors.Is(err, errTunClosed) {
		t.Fatalf("inject после Close: %v, ожидалось errTunClosed", err)
	}
}

// EventUp не подкладывается: Up() вызывается явно после IpcSet.
func TestPktTunNoEventUp(t *testing.T) {
	tn := newPktTun(1420, 8, func([]byte) {})
	defer tn.Close()
	select {
	case ev := <-tn.Events():
		t.Fatalf("в канале событий лежит %v, ожидалось пусто", ev)
	default:
	}
}

// Полная очередь — пакет отбрасывается с ошибкой, Read не блокируется навсегда.
func TestPktTunQueueFull(t *testing.T) {
	tn := newPktTun(1420, 2, func([]byte) {})
	defer tn.Close()
	for i := 0; i < 2; i++ {
		if err := tn.inject(samplePacket(30)); err != nil {
			t.Fatalf("inject %d: %v", i, err)
		}
	}
	if err := tn.inject(samplePacket(30)); !errors.Is(err, errQueueFull) {
		t.Fatalf("третий inject: %v, ожидалось errQueueFull", err)
	}
}

// BatchSize = 1 и MTU из конструктора.
func TestPktTunBatchAndMTU(t *testing.T) {
	tn := newPktTun(1376, 8, func([]byte) {})
	defer tn.Close()
	if tn.BatchSize() != 1 {
		t.Fatalf("BatchSize=%d", tn.BatchSize())
	}
	if mtu, err := tn.MTU(); err != nil || mtu != 1376 {
		t.Fatalf("MTU=%d err=%v", mtu, err)
	}
}

// resync будит Read без пакета: ядро перечитывает S4 и возвращается с новым смещением.
func TestPktTunResyncWakesReadWithoutPacket(t *testing.T) {
	tn := newPktTun(1420, 8, func([]byte) {})
	defer tn.Close()
	type result struct {
		n   int
		err error
	}
	done := make(chan result, 1)
	go func() {
		buf := make([]byte, 100)
		n, err := tn.Read([][]byte{buf}, make([]int, 1), 16)
		done <- result{n, err}
	}()
	time.Sleep(20 * time.Millisecond)
	tn.resync()
	select {
	case r := <-done:
		if r.n != 0 || r.err != nil {
			t.Fatalf("Read после resync: n=%d err=%v, ожидалось 0, nil", r.n, r.err)
		}
	case <-time.After(2 * time.Second):
		t.Fatalf("Read не проснулся после resync")
	}
	// Следующий Read работает как обычно и получает пакет по новому смещению.
	pkt := samplePacket(30)
	if err := tn.inject(pkt); err != nil {
		t.Fatalf("inject: %v", err)
	}
	buf := make([]byte, 100)
	sizes := make([]int, 1)
	if n, err := tn.Read([][]byte{buf}, sizes, 41); err != nil || n != 1 || !bytes.Equal(buf[41:41+sizes[0]], pkt) {
		t.Fatalf("Read после resync: n=%d err=%v", n, err)
	}
}
