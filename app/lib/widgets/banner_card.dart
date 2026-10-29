import 'package:flutter/material.dart';

/// Плашка над кнопкой: значок, заголовок, пояснение, действия. Действия
/// переносятся вниз, поэтому на узком экране ничего не обрезается.
class BannerCard extends StatelessWidget {
  const BannerCard({
    super.key,
    required this.icon,
    required this.title,
    this.subtitle,
    this.actions = const [],
    this.onClose,
    this.closeKey,
    this.isProblem = false,
  });

  final IconData icon;
  final String title;
  final String? subtitle;
  final List<Widget> actions;

  /// Крестик справа; `null` — закрыть нельзя.
  final VoidCallback? onClose;
  final Key? closeKey;
  final bool isProblem;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final foreground = isProblem ? scheme.onErrorContainer : scheme.onSurface;
    return Card(
      color: isProblem ? scheme.errorContainer : null,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Padding(
                  padding: const EdgeInsets.only(top: 2),
                  child: Icon(icon, color: foreground),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        style: theme.textTheme.titleSmall?.copyWith(
                          color: foreground,
                        ),
                      ),
                      if (subtitle != null) ...[
                        const SizedBox(height: 2),
                        Text(
                          subtitle!,
                          style: theme.textTheme.bodySmall?.copyWith(
                            color: isProblem
                                ? scheme.onErrorContainer
                                : scheme.onSurfaceVariant,
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
                if (onClose != null)
                  IconButton(
                    key: closeKey,
                    visualDensity: VisualDensity.compact,
                    tooltip: 'Закрыть',
                    icon: const Icon(Icons.close),
                    onPressed: onClose,
                  ),
              ],
            ),
            if (actions.isNotEmpty)
              Align(
                alignment: Alignment.centerRight,
                child: Wrap(spacing: 4, children: actions),
              ),
          ],
        ),
      ),
    );
  }
}
