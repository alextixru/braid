import 'package:braid/screens/home/home_status.dart';
import 'package:flutter/material.dart';

/// Большая круглая кнопка подключения: цвет и значок говорят о состоянии, а
/// подпись внутри — что будет по нажатию.
class ConnectButton extends StatelessWidget {
  const ConnectButton({
    super.key,
    required this.label,
    required this.tone,
    required this.onPressed,
  });

  final String label;
  final StatusTone tone;

  /// `null` — кнопка занята.
  final VoidCallback? onPressed;

  static const double size = 168;

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    final (background, foreground) = switch (tone) {
      StatusTone.idle => (scheme.surfaceContainerHighest, scheme.onSurface),
      StatusTone.working => (
        scheme.primaryContainer,
        scheme.onPrimaryContainer,
      ),
      StatusTone.ok => (scheme.primary, scheme.onPrimary),
      StatusTone.problem => (scheme.errorContainer, scheme.onErrorContainer),
      StatusTone.warning => (
        scheme.tertiaryContainer,
        scheme.onTertiaryContainer,
      ),
    };
    return Semantics(
      button: true,
      enabled: onPressed != null,
      label: label,
      onTap: onPressed,
      excludeSemantics: true,
      child: SizedBox.square(
        dimension: size,
        child: Material(
          key: const Key('connect-button'),
          color: background,
          shape: const CircleBorder(),
          clipBehavior: Clip.antiAlias,
          child: InkWell(
            onTap: onPressed,
            child: Stack(
              alignment: Alignment.center,
              children: [
                if (tone == StatusTone.working)
                  SizedBox.square(
                    dimension: size - 12,
                    child: CircularProgressIndicator(
                      strokeWidth: 3,
                      color: foreground.withValues(alpha: 0.5),
                    ),
                  ),
                Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(Icons.power_settings_new, size: 56, color: foreground),
                    const SizedBox(height: 6),
                    Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 16),
                      child: Text(
                        label,
                        textAlign: TextAlign.center,
                        maxLines: 2,
                        style: Theme.of(context).textTheme.titleSmall?.copyWith(
                          color: foreground,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
