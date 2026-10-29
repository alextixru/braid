package main

import (
	"errors"

	"github.com/amnezia-vpn/amneziawg-go/v3/conn"
)

var errProtect = errors.New("libawg: платформа отказалась защитить сокет")

// protectBind — conn.Bind, который защищает свои UDP-сокеты при открытии
// (Android: VpnService.protect(fd), иначе сокеты ушли бы в наш же туннель).
// Send начинается только после возврата Open, а каждый BindUpdate зовёт Open
// заново — перезащита при смене сети автоматическая (notes-awg.md §1.5).
type protectBind struct {
	conn.Bind
	protect func(fd int) bool // nil — защита не нужна (macOS)
}

func (b *protectBind) Open(port uint16) ([]conn.ReceiveFunc, uint16, error) {
	fns, actual, err := b.Bind.Open(port)
	if err != nil {
		return nil, 0, err
	}
	if b.protect == nil {
		return fns, actual, nil
	}
	peek, ok := b.Bind.(conn.PeekLookAtSocketFd)
	if !ok {
		return fns, actual, nil
	}
	for _, get := range []func() (int, error){peek.PeekLookAtSocketFd4, peek.PeekLookAtSocketFd6} {
		fd, err := get()
		if err != nil || fd < 0 {
			continue // сокета этого семейства нет (например, IPv6 выключен)
		}
		if !b.protect(fd) {
			b.Bind.Close()
			return nil, 0, errProtect
		}
	}
	return fns, actual, nil
}
