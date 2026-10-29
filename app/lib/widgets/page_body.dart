import 'package:flutter/material.dart';

/// Содержимое экрана: прокручивается и не растягивается шире читаемого —
/// на окне macOS колонка стоит по центру, на телефоне занимает всю ширину.
class PageBody extends StatelessWidget {
  const PageBody({
    super.key,
    required this.children,
    this.maxWidth = 560,
    this.padding = const EdgeInsets.fromLTRB(16, 16, 16, 32),
  });

  final List<Widget> children;
  final double maxWidth;
  final EdgeInsetsGeometry padding;

  @override
  Widget build(BuildContext context) {
    return Align(
      alignment: Alignment.topCenter,
      child: ConstrainedBox(
        constraints: BoxConstraints(maxWidth: maxWidth),
        child: ListView(padding: padding, children: children),
      ),
    );
  }
}
