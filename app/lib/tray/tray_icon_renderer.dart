/// Картинка значка в меню-баре (ист. 56): круг цвета живости с буквой движка.
///
/// Рисуется на лету, а не лежит файлами в assets: движков (вид приходит из
/// подписки) и состояний больше, чем стоит заготавливать картинок. Готовые
/// PNG кэшируются по паре «глиф + цвет».
library;

import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:braid/tray/tray_status.dart';
import 'package:flutter/painting.dart';

abstract interface class TrayIconRenderer {
  /// PNG значка для [status].
  Future<Uint8List> render(TrayStatus status);
}

class CanvasTrayIconRenderer implements TrayIconRenderer {
  /// Сторона в пикселях: значок в меню-баре — 18 pt, на Retina это 36 px
  /// (размер в pt задаёт `tray_manager`, по умолчанию 18).
  static const size = 36;

  /// Цвета живости — системные цвета macOS.
  static const colors = {
    TrayTone.idle: ui.Color(0xFF8E8E93),
    TrayTone.working: ui.Color(0xFF0A84FF),
    TrayTone.ok: ui.Color(0xFF34C759),
    TrayTone.warning: ui.Color(0xFFFF9F0A),
    TrayTone.problem: ui.Color(0xFFFF3B30),
  };

  final _cache = <String, Uint8List>{};

  @override
  Future<Uint8List> render(TrayStatus status) async {
    final cached = _cache[status.iconKey];
    if (cached != null) return cached;

    const side = size * 1.0;
    final recorder = ui.PictureRecorder();
    final canvas = ui.Canvas(
      recorder,
      const ui.Rect.fromLTWH(0, 0, side, side),
    );
    canvas.drawCircle(
      const ui.Offset(side / 2, side / 2),
      side / 2 - 2,
      ui.Paint()
        ..color = colors[status.tone]!
        ..isAntiAlias = true,
    );
    final letter = TextPainter(
      text: TextSpan(
        text: status.glyph,
        style: const TextStyle(
          color: ui.Color(0xFFFFFFFF),
          fontSize: 22,
          fontWeight: FontWeight.w700,
          height: 1,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    letter.paint(
      canvas,
      ui.Offset((side - letter.width) / 2, (side - letter.height) / 2),
    );
    letter.dispose();

    final picture = recorder.endRecording();
    final image = await picture.toImage(size, size);
    picture.dispose();
    final data = await image.toByteData(format: ui.ImageByteFormat.png);
    image.dispose();
    final png = data!.buffer.asUint8List();
    _cache[status.iconKey] = png;
    return png;
  }
}
