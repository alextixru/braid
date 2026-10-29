#!/usr/bin/env python3
import http.server
import select
import socket
import sys
from datetime import datetime
from pathlib import Path

PORT = 19999
# Журнал — в archive/ (вне git), а не в домашнем каталоге с именем пользователя
LOG_FILE = str(Path(__file__).resolve().parent.parent / "archive" / "cursor_requests.log")

class InterceptHandler(http.server.BaseHTTPRequestHandler):
    def do_CONNECT(self):
        host, _, port = self.path.partition(":")
        port = int(port) if port else 443
        ts = datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
        line = f"[{ts}] CONNECT -> {host}:{port}"
        print(line, flush=True)
        try:
            with open(LOG_FILE, "a") as f:
                f.write(line + "\n")
        except Exception:
            pass

        try:
            target = socket.create_connection((host, port), timeout=10)
            self.send_response(200, "Connection Established")
            self.end_headers()
        except Exception as e:
            self.send_error(502, f"Bad Gateway: {e}")
            return

        conns = [self.connection, target]
        try:
            while True:
                rlist, _, xlist = select.select(conns, [], conns, 30)
                if xlist or not rlist:
                    break
                for s in rlist:
                    other = target if s is self.connection else self.connection
                    data = s.recv(8192)
                    if not data:
                        return
                    other.sendall(data)
        finally:
            target.close()

    def log_message(self, format, *args):
        pass

if __name__ == "__main__":
    print(f"=== Cursor Traffic Sniffer запущен на 127.0.0.1:{PORT} ===", flush=True)
    print(f"Логи также пишутся в {LOG_FILE}", flush=True)
    print("Нажмите Ctrl+C для завершения.", flush=True)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", PORT), InterceptHandler)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nSniffer остановлен.")
