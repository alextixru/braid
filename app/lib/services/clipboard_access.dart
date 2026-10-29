import 'package:flutter/services.dart';

/// Буфер обмена. Интерфейс нужен тестам: системный канал в виджет-тестах
/// молчит, а «+ Домен» и «Вставить» читают именно его.
abstract interface class ClipboardAccess {
  Future<String?> readText();
  Future<void> writeText(String text);
}

class SystemClipboard implements ClipboardAccess {
  const SystemClipboard();

  @override
  Future<String?> readText() async =>
      (await Clipboard.getData(Clipboard.kTextPlain))?.text;

  @override
  Future<void> writeText(String text) =>
      Clipboard.setData(ClipboardData(text: text));
}
