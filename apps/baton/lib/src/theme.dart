// SPDX-License-Identifier: Apache-2.0

import 'package:flutter/cupertino.dart';
import 'package:flutter/material.dart';

/// Baton's own look: one signature colour on calm, warm neutrals, soft corners, flat cards, and
/// the motion and type of the system it runs on. Every screen takes its colours, shapes and type
/// from here, so the look changes in one place.
abstract final class BatonTheme {
  /// The signature colour, a warm ember. It marks what can be pressed and what is live, and
  /// little else.
  static const ember = Color(0xFFE2571E);

  /// The ember for filled buttons and text on light surfaces: dark enough for white text and for
  /// text on white to read at 4.5:1 or more.
  static const _emberLight = Color(0xFFB8420F);

  /// The ember on dark surfaces: light enough to read on near black, with dark text on top.
  static const _emberDark = Color(0xFFFF8A4C);

  /// Room between things, in steps that every screen uses.
  static const gapSmall = 8.0;
  static const gap = 16.0;
  static const gapLarge = 24.0;

  /// How round corners are: cards and sheets, then buttons and fields.
  static const radiusLarge = 20.0;
  static const radius = 14.0;

  static ThemeData light() => _build(Brightness.light);

  static ThemeData dark() => _build(Brightness.dark);

  static ColorScheme _scheme(Brightness brightness) {
    final light = brightness == Brightness.light;
    final base = ColorScheme.fromSeed(
      seedColor: ember,
      brightness: brightness,
      dynamicSchemeVariant: DynamicSchemeVariant.fidelity,
    );
    return base.copyWith(
      primary: light ? _emberLight : _emberDark,
      onPrimary: light ? Colors.white : const Color(0xFF2A0E00),
      // Warm greys, close to the system's own, so Baton sits well beside other apps.
      surface: light ? const Color(0xFFF6F5F3) : const Color(0xFF0E0E10),
      onSurface: light ? const Color(0xFF1C1B1A) : const Color(0xFFF2F1EF),
      onSurfaceVariant: light
          ? const Color(0xFF5F5C58)
          : const Color(0xFFA9A6A1),
      surfaceContainerLowest: light ? Colors.white : const Color(0xFF151517),
      surfaceContainerLow: light ? Colors.white : const Color(0xFF1A1A1C),
      surfaceContainer: light ? Colors.white : const Color(0xFF1E1E21),
      surfaceContainerHigh: light
          ? const Color(0xFFEFEDEA)
          : const Color(0xFF26262A),
      surfaceContainerHighest: light
          ? const Color(0xFFE7E4E0)
          : const Color(0xFF2E2E33),
      outline: light ? const Color(0xFF8C8883) : const Color(0xFF77746F),
      outlineVariant: light ? const Color(0xFFE3E0DC) : const Color(0xFF2F2F33),
      surfaceTint: Colors.transparent,
    );
  }

  static ThemeData _build(Brightness brightness) {
    final scheme = _scheme(brightness);
    final base = ThemeData(
      colorScheme: scheme,
      brightness: brightness,
      useMaterial3: true,
    );
    final text = base.textTheme;
    final cardShape = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(radiusLarge),
      side: BorderSide(color: scheme.outlineVariant),
    );
    final buttonShape = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(radius),
    );
    const buttonSize = Size(64, 48);
    const buttonPadding = EdgeInsets.symmetric(horizontal: 20);
    final buttonText = text.labelLarge?.copyWith(
      fontWeight: FontWeight.w600,
      fontSize: 16,
    );
    return base.copyWith(
      scaffoldBackgroundColor: scheme.surface,
      // Headings are bolder and tighter, body text a touch larger, as on Apple devices.
      textTheme: text.copyWith(
        displaySmall: text.displaySmall?.copyWith(
          fontWeight: FontWeight.w700,
          letterSpacing: -0.5,
        ),
        headlineMedium: text.headlineMedium?.copyWith(
          fontWeight: FontWeight.w700,
          letterSpacing: -0.4,
        ),
        headlineSmall: text.headlineSmall?.copyWith(
          fontWeight: FontWeight.w700,
          letterSpacing: -0.3,
        ),
        titleLarge: text.titleLarge?.copyWith(
          fontWeight: FontWeight.w600,
          letterSpacing: -0.2,
        ),
        titleMedium: text.titleMedium?.copyWith(fontWeight: FontWeight.w600),
        bodyLarge: text.bodyLarge?.copyWith(fontSize: 17, height: 1.35),
        bodyMedium: text.bodyMedium?.copyWith(
          fontSize: 15,
          height: 1.35,
          color: scheme.onSurfaceVariant,
        ),
        labelMedium: text.labelMedium?.copyWith(
          color: scheme.onSurfaceVariant,
          letterSpacing: 0.2,
        ),
      ),
      // The way from one screen to the next is the system's own on Apple devices.
      pageTransitionsTheme: const PageTransitionsTheme(
        builders: {
          TargetPlatform.iOS: CupertinoPageTransitionsBuilder(),
          TargetPlatform.macOS: CupertinoPageTransitionsBuilder(),
          TargetPlatform.android: ZoomPageTransitionsBuilder(),
        },
      ),
      appBarTheme: AppBarTheme(
        backgroundColor: scheme.surface,
        foregroundColor: scheme.onSurface,
        elevation: 0,
        scrolledUnderElevation: 0,
        surfaceTintColor: Colors.transparent,
        titleTextStyle: text.titleLarge?.copyWith(
          fontWeight: FontWeight.w700,
          color: scheme.onSurface,
        ),
      ),
      cardTheme: CardThemeData(
        color: scheme.surfaceContainer,
        elevation: 0,
        shape: cardShape,
        clipBehavior: Clip.antiAlias,
      ),
      listTileTheme: ListTileThemeData(
        contentPadding: const EdgeInsets.symmetric(horizontal: gap),
        iconColor: scheme.onSurfaceVariant,
        titleTextStyle: text.bodyLarge?.copyWith(
          fontWeight: FontWeight.w600,
          color: scheme.onSurface,
        ),
        subtitleTextStyle: text.bodyMedium?.copyWith(
          fontSize: 14,
          color: scheme.onSurfaceVariant,
        ),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          minimumSize: buttonSize,
          padding: buttonPadding,
          shape: buttonShape,
          textStyle: buttonText,
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          minimumSize: buttonSize,
          padding: buttonPadding,
          shape: buttonShape,
          textStyle: buttonText,
          side: BorderSide(color: scheme.outlineVariant),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style: TextButton.styleFrom(shape: buttonShape, textStyle: buttonText),
      ),
      floatingActionButtonTheme: FloatingActionButtonThemeData(
        backgroundColor: scheme.primary,
        foregroundColor: scheme.onPrimary,
        elevation: 0,
        focusElevation: 0,
        hoverElevation: 1,
        highlightElevation: 0,
        shape: const StadiumBorder(),
        extendedTextStyle: buttonText,
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: scheme.surfaceContainerHigh,
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(radius),
          borderSide: BorderSide.none,
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(radius),
          borderSide: BorderSide.none,
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(radius),
          borderSide: BorderSide(color: scheme.primary, width: 2),
        ),
      ),
      navigationBarTheme: NavigationBarThemeData(
        backgroundColor: scheme.surfaceContainerLow,
        surfaceTintColor: Colors.transparent,
        elevation: 0,
        height: 68,
        indicatorColor: scheme.primary.withValues(alpha: 0.14),
        iconTheme: WidgetStateProperty.resolveWith(
          (states) => IconThemeData(
            color: states.contains(WidgetState.selected)
                ? scheme.primary
                : scheme.onSurfaceVariant,
          ),
        ),
        labelTextStyle: WidgetStateProperty.resolveWith(
          (states) => text.labelMedium?.copyWith(
            fontWeight: FontWeight.w600,
            color: states.contains(WidgetState.selected)
                ? scheme.primary
                : scheme.onSurfaceVariant,
          ),
        ),
      ),
      dialogTheme: DialogThemeData(
        backgroundColor: scheme.surfaceContainer,
        surfaceTintColor: Colors.transparent,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(radiusLarge + 4),
        ),
      ),
      bottomSheetTheme: BottomSheetThemeData(
        backgroundColor: scheme.surfaceContainer,
        surfaceTintColor: Colors.transparent,
        showDragHandle: true,
        shape: const RoundedRectangleBorder(
          borderRadius: BorderRadius.vertical(
            top: Radius.circular(radiusLarge + 8),
          ),
        ),
      ),
      popupMenuTheme: PopupMenuThemeData(
        color: scheme.surfaceContainer,
        surfaceTintColor: Colors.transparent,
        elevation: 6,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(radius),
        ),
      ),
      snackBarTheme: SnackBarThemeData(
        behavior: SnackBarBehavior.floating,
        backgroundColor: scheme.inverseSurface,
        actionTextColor: scheme.inversePrimary,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(radius),
        ),
      ),
      dividerTheme: DividerThemeData(
        color: scheme.outlineVariant,
        space: 1,
        thickness: 1,
      ),
      progressIndicatorTheme: ProgressIndicatorThemeData(
        color: scheme.primary,
        linearTrackColor: scheme.surfaceContainerHighest,
      ),
    );
  }
}
