package main

import (
	"errors"
	"syscall"
	"testing"

	"github.com/amnezia-vpn/amneziawg-go/v3/conn"
)

// Шов protect (notes-awg.md §1.5, вариант 2): сокеты защищаются внутри Open,
// до первого Send, и заново при каждом BindUpdate. Тест открывает настоящий
// UDP-сокет на случайном порту, ничего никуда не шлёт.

func TestProtectBindOpenProtectsSockets(t *testing.T) {
	var fds []int
	b := &protectBind{
		Bind: conn.NewDefaultBind(),
		protect: func(fd int) bool {
			fds = append(fds, fd)
			return true
		},
	}
	fns, port, err := b.Open(0)
	if err != nil {
		t.Fatalf("Open: %v", err)
	}
	defer b.Close()
	if port == 0 || len(fns) == 0 {
		t.Fatalf("Open: port=%d, fns=%d", port, len(fns))
	}
	if len(fds) == 0 {
		t.Fatalf("protect не вызван ни для одного сокета")
	}
	for _, fd := range fds {
		typ, err := syscall.GetsockoptInt(fd, syscall.SOL_SOCKET, syscall.SO_TYPE)
		if err != nil || typ != syscall.SOCK_DGRAM {
			t.Fatalf("fd %d — не UDP-сокет: type=%d err=%v", fd, typ, err)
		}
	}
}

func TestProtectBindOpenFailsWhenProtectRefuses(t *testing.T) {
	refused := false
	b := &protectBind{
		Bind:    conn.NewDefaultBind(),
		protect: func(int) bool { return refused },
	}
	if _, _, err := b.Open(0); !errors.Is(err, errProtect) {
		t.Fatalf("Open при отказе protect: %v, ожидалось errProtect", err)
	}
	// После отказа сокеты закрыты: повторное открытие возможно.
	refused = true
	if _, _, err := b.Open(0); err != nil {
		t.Fatalf("повторный Open: %v", err)
	}
	b.Close()
}

// Bind без PeekLookAtSocketFd (например, канальный в тестах) открывается без protect.
type plainBind struct{ conn.Bind }

func (p plainBind) Open(port uint16) ([]conn.ReceiveFunc, uint16, error) {
	return []conn.ReceiveFunc{}, 1, nil
}

func TestProtectBindWithoutPeekSkipsProtect(t *testing.T) {
	called := false
	b := &protectBind{Bind: plainBind{}, protect: func(int) bool { called = true; return false }}
	if _, port, err := b.Open(0); err != nil || port != 1 {
		t.Fatalf("Open: port=%d err=%v", port, err)
	}
	if called {
		t.Fatalf("protect вызван для Bind без сокетов")
	}
}
