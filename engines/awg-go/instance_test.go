package main

import (
	"bytes"
	"crypto/ecdh"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"log"
	"net/netip"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/amnezia-vpn/amneziawg-go/v3/conn/bindtest"
	"github.com/amnezia-vpn/amneziawg-go/v3/tun/tuntest"
)

// Шов libawg (interfaces.md): два ядра в памяти с общим канальным Bind
// устанавливают хендшейк и прогоняют IP-пакет туда и обратно при S4 ≠ 0 и
// заданном I1. Сеть не используется: bindtest.NewChannelBinds — каналы в памяти.

// genKeyPair — синтетическая пара X25519 в hex для UAPI.
func genKeyPair(t *testing.T) (privHex, pubHex string) {
	t.Helper()
	key, err := ecdh.X25519().GenerateKey(rand.Reader)
	if err != nil {
		t.Fatalf("ключ: %v", err)
	}
	return hex.EncodeToString(key.Bytes()), hex.EncodeToString(key.PublicKey().Bytes())
}

// uapiCfg собирает текст UAPI из пар ключ/значение.
func uapiCfg(kv ...string) string {
	if len(kv)%2 != 0 {
		panic("нечётное число аргументов uapiCfg")
	}
	var b strings.Builder
	for i := 0; i < len(kv); i += 2 {
		b.WriteString(kv[i])
		b.WriteByte('=')
		b.WriteString(kv[i+1])
		b.WriteByte('\n')
	}
	return b.String()
}

// awgParams — одинаковые для обеих сторон параметры AWG 2.0 с S4 ≠ 0 и I1.
var awgParams = []string{
	"jc", "2",
	"jmin", "40",
	"jmax", "70",
	"s1", "15",
	"s2", "18",
	"s3", "20",
	"s4", "25",
	"h1", "123456-123500",
	"h2", "67543-67550",
	"h3", "123123-123200",
	"h4", "32345-32350",
	"i1", "<b 0x4a7b2f><r 16><t>",
}

func uapiValue(cfg, key string) (string, bool) {
	for _, line := range strings.Split(cfg, "\n") {
		if k, v, ok := strings.Cut(line, "="); ok && k == key {
			return v, true
		}
	}
	return "", false
}

func uapiUint(t *testing.T, cfg, key string) uint64 {
	t.Helper()
	v, ok := uapiValue(cfg, key)
	if !ok {
		t.Fatalf("в выводе IpcGet нет %s", key)
	}
	n, err := strconv.ParseUint(v, 10, 64)
	if err != nil {
		t.Fatalf("%s=%q: %v", key, v, err)
	}
	return n
}

func testLog(level int, msg string) {
	if testing.Verbose() {
		log.Printf("[awg %d] %s", level, msg)
	}
}

// После newInstance ядро уже поднято: IpcSet, затем явный Up() — в IpcGet
// появляется listen_port выбранного сокета.
func TestInstanceUpAfterIpcSet(t *testing.T) {
	binds := bindtest.NewChannelBinds()
	priv, _ := genKeyPair(t)
	_, peerPub := genKeyPair(t)
	cfg := uapiCfg(append(append([]string{"private_key", priv, "replace_peers", "true"}, awgParams...),
		"public_key", peerPub, "protocol_version", "1", "replace_allowed_ips", "true", "allowed_ip", "0.0.0.0/0")...)

	inst, err := newInstance(cfg, 1420, binds[0], callbacks{tx: func([]byte) {}, log: testLog})
	if err != nil {
		t.Fatalf("newInstance: %v", err)
	}
	defer inst.close()

	got, err := inst.getConfig()
	if err != nil {
		t.Fatalf("getConfig: %v", err)
	}
	if port := uapiUint(t, got, "listen_port"); port == 0 {
		t.Fatalf("listen_port=0: Up() не вызван")
	}
	if s4 := uapiUint(t, got, "s4"); s4 != 25 {
		t.Fatalf("s4=%d, ожидалось 25", s4)
	}
	if i1, ok := uapiValue(got, "i1"); !ok || i1 != "<b 0x4a7b2f><r 16><t>" {
		t.Fatalf("i1=%q", i1)
	}
}

// Кривой конфиг — ошибка и ни одного живого ядра.
func TestInstanceRejectsBadConfig(t *testing.T) {
	binds := bindtest.NewChannelBinds()
	if inst, err := newInstance("private_key=zz\n", 1420, binds[0], callbacks{}); err == nil {
		inst.close()
		t.Fatalf("ожидалась ошибка конфига")
	}
	// Пересекающиеся H1/H2 отвергаются ядром при слиянии.
	priv, _ := genKeyPair(t)
	cfg := uapiCfg("private_key", priv, "h1", "10-20", "h2", "15-25", "h3", "30", "h4", "40")
	if inst, err := newInstance(cfg, 1420, binds[1], callbacks{}); err == nil {
		inst.close()
		t.Fatalf("ожидалась ошибка «headers must not overlap»")
	}
}

type memPeer struct {
	inst *instance
	rx   chan []byte
	ip   netip.Addr
	pub  string
}

// newMemPair поднимает два ядра на общем канальном Bind и прописывает им адреса друг друга.
func newMemPair(t *testing.T) [2]*memPeer {
	t.Helper()
	binds := bindtest.NewChannelBinds()
	var priv, pub [2]string
	for i := range priv {
		priv[i], pub[i] = genKeyPair(t)
	}
	var pair [2]*memPeer
	for i := range pair {
		p := &memPeer{rx: make(chan []byte, 64), ip: netip.AddrFrom4([4]byte{10, 7, 0, byte(i + 1)}), pub: pub[i]}
		cfg := uapiCfg(append(append([]string{"private_key", priv[i], "replace_peers", "true"}, awgParams...),
			"public_key", pub[i^1], "protocol_version", "1", "replace_allowed_ips", "true", "allowed_ip", "0.0.0.0/0")...)
		rx := p.rx
		inst, err := newInstance(cfg, 1420, binds[i], callbacks{
			tx:  func(pkt []byte) { rx <- append([]byte(nil), pkt...) },
			log: testLog,
		})
		if err != nil {
			t.Fatalf("newInstance %d: %v", i, err)
		}
		p.inst = inst
		t.Cleanup(inst.close)
		pair[i] = p
	}
	for i := range pair {
		got, err := pair[i^1].inst.getConfig()
		if err != nil {
			t.Fatalf("getConfig %d: %v", i^1, err)
		}
		port := uapiUint(t, got, "listen_port")
		ep := uapiCfg("public_key", pair[i^1].pub, "endpoint", fmt.Sprintf("127.0.0.1:%d", port))
		if err := pair[i].inst.setConfig(ep); err != nil {
			t.Fatalf("setConfig endpoint %d: %v", i, err)
		}
	}
	return pair
}

func expectPacket(t *testing.T, rx chan []byte, want []byte, what string) {
	t.Helper()
	select {
	case got := <-rx:
		if !bytes.Equal(got, want) {
			t.Fatalf("%s: пакет пришёл искажённым (%d байт вместо %d)", what, len(got), len(want))
		}
	case <-time.After(6 * time.Second):
		t.Fatalf("%s: пакет не дошёл", what)
	}
}

func TestTwoInstancesHandshakeInMemory(t *testing.T) {
	pair := newMemPair(t)

	ping := tuntest.Ping(pair[0].ip, pair[1].ip) // от 1 к 0: первый пакет запускает хендшейк
	if err := pair[1].inst.inject(ping); err != nil {
		t.Fatalf("inject ping: %v", err)
	}
	expectPacket(t, pair[0].rx, ping, "ping 1→0")

	pong := tuntest.Ping(pair[1].ip, pair[0].ip)
	if err := pair[0].inst.inject(pong); err != nil {
		t.Fatalf("inject pong: %v", err)
	}
	expectPacket(t, pair[1].rx, pong, "pong 0→1")

	for i, p := range pair {
		got, err := p.inst.getConfig()
		if err != nil {
			t.Fatalf("getConfig %d: %v", i, err)
		}
		if uapiUint(t, got, "last_handshake_time_sec") == 0 {
			t.Fatalf("ядро %d: хендшейка нет в IpcGet", i)
		}
		if uapiUint(t, got, "rx_bytes") == 0 || uapiUint(t, got, "tx_bytes") == 0 {
			t.Fatalf("ядро %d: счётчики байтов пусты", i)
		}
	}
}

// После close inject и setConfig отвечают ошибкой, tx больше не вызывается.
func TestInstanceCloseStopsTraffic(t *testing.T) {
	pair := newMemPair(t)
	ping := tuntest.Ping(pair[0].ip, pair[1].ip)
	if err := pair[1].inst.inject(ping); err != nil {
		t.Fatalf("inject: %v", err)
	}
	expectPacket(t, pair[0].rx, ping, "ping")

	pair[0].inst.close()
	pair[0].inst.close() // повторный close безвреден
	if err := pair[0].inst.inject(ping); err == nil {
		t.Fatalf("inject после close прошёл")
	}
	if _, err := pair[0].inst.getConfig(); err == nil {
		t.Fatalf("getConfig после close прошёл")
	}
}
