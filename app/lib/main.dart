import 'package:braid/app.dart';
import 'package:braid/src/rust/api/braid.dart';
import 'package:braid/src/rust/frb_generated.dart';
import 'package:flutter/material.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLib.init();
  runApp(BraidApp(coreVersion: coreVersion()));
}
