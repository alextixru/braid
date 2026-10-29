// Картинка значка (ист. 56): круг цвета живости с буквой движка. Рисуется
// на лету: движков и состояний больше, чем стоит заготавливать файлов.

import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:braid/tray/tray_icon_renderer.dart';
import 'package:braid/tray/tray_status.dart';
import 'package:flutter_test/flutter_test.dart';

TrayStatus status(String glyph, TrayTone tone) =>
    TrayStatus(glyph: glyph, tone: tone, headline: 'проверка');

/// Картинка PNG → ширина, высота и пиксели RGBA.
Future<({int width, int height, Uint8List rgba})> decode(Uint8List png) async {
  final codec = await ui.instantiateImageCodec(png);
  final frame = await codec.getNextFrame();
  final image = frame.image;
  final data = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
  return (
    width: image.width,
    height: image.height,
    rgba: data!.buffer.asUint8List(),
  );
}

List<int> pixel(({int width, int height, Uint8List rgba}) image, int x, int y) {
  final at = (y * image.width + x) * 4;
  return image.rgba.sublist(at, at + 4);
}

void main() {
  const signature = [0x89, 0x50, 0x4E, 0x47];

  testWidgets('PNG 36×36: 18 pt на экране Retina', (tester) async {
    final png = await tester.runAsync(
      () => CanvasTrayIconRenderer().render(status('A', TrayTone.ok)),
    );
    expect(png!.sublist(0, 4), signature);
    final image = await tester.runAsync(() => decode(png));
    expect(image!.width, CanvasTrayIconRenderer.size);
    expect(image.height, CanvasTrayIconRenderer.size);
  });

  testWidgets('круг закрашен цветом живости, углы прозрачные', (tester) async {
    final renderer = CanvasTrayIconRenderer();
    Future<List<int>> edge(TrayTone tone) async {
      final png = await renderer.render(status('A', tone));
      return pixel(await decode(png), 4, 18);
    }

    final ok = await tester.runAsync(() => edge(TrayTone.ok));
    final problem = await tester.runAsync(() => edge(TrayTone.problem));
    final idle = await tester.runAsync(() => edge(TrayTone.idle));
    expect(ok, [0x34, 0xC7, 0x59, 0xFF], reason: 'зелёный');
    expect(problem, [0xFF, 0x3B, 0x30, 0xFF], reason: 'красный');
    expect(idle, [0x8E, 0x8E, 0x93, 0xFF], reason: 'серый');

    final png = await tester.runAsync(
      () => renderer.render(status('A', TrayTone.ok)),
    );
    final corner = pixel((await tester.runAsync(() => decode(png!)))!, 0, 0);
    expect(corner[3], 0, reason: 'вне круга пиксель прозрачный');
  });

  testWidgets('буква нарисована поверх круга', (tester) async {
    final png = await tester.runAsync(
      () => CanvasTrayIconRenderer().render(status('A', TrayTone.ok)),
    );
    final image = (await tester.runAsync(() => decode(png!)))!;
    var other = 0;
    for (var at = 0; at < image.rgba.length; at += 4) {
      final isFill =
          image.rgba[at] == 0x34 &&
          image.rgba[at + 1] == 0xC7 &&
          image.rgba[at + 2] == 0x59;
      if (image.rgba[at + 3] == 0xFF && !isFill) other++;
    }
    expect(other, greaterThan(10), reason: 'белые пиксели буквы');
  });

  testWidgets('одинаковое состояние рисуется один раз', (tester) async {
    final renderer = CanvasTrayIconRenderer();
    final first = await tester.runAsync(
      () => renderer.render(status('A', TrayTone.ok)),
    );
    final second = await tester.runAsync(
      () => renderer.render(status('A', TrayTone.ok)),
    );
    expect(identical(first, second), isTrue);
  });
}
