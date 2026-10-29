import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:qr_flutter/qr_flutter.dart';

/// QR ссылки подписки. Белый фон с полем — иначе на тёмной теме код не
/// читается; сторона не больше [size] и не шире доступного места.
class LinkQr extends StatelessWidget {
  const LinkQr({super.key, required this.data, this.size = 240});

  /// Текст ссылки `braid://…` — секрет, показывается только по действию пользователя.
  final String data;
  final double size;

  static const _frame = 12.0;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final room = constraints.maxWidth.isFinite
            ? constraints.maxWidth - 2 * _frame
            : size;
        final side = math.min(size, math.max(room, 0.0));
        return DecoratedBox(
          decoration: BoxDecoration(
            color: Colors.white,
            borderRadius: BorderRadius.circular(12),
          ),
          child: Padding(
            padding: const EdgeInsets.all(_frame),
            child: QrImageView(
              data: data,
              size: side,
              backgroundColor: Colors.white,
              errorCorrectionLevel: QrErrorCorrectLevel.M,
              semanticsLabel: 'QR-код ссылки подписки',
            ),
          ),
        );
      },
    );
  }
}
