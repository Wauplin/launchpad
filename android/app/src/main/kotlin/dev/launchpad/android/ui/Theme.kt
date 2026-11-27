package dev.launchpad.android.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/** Launchpad's colours, the web app's tokens (web/src/styles.css): paper, ink, teal, amber. */
data class LpColors(
    val board: Color, val paper: Color, val chrome: Color, val sunk: Color,
    val ink: Color, val ink2: Color, val ink3: Color, val line: Color, val line2: Color,
    val teal: Color, val tealInk: Color, val tealSoft: Color,
    val attn: Color, val attnSoft: Color, val attnLine: Color,
    val work: Color, val workSoft: Color, val ok: Color, val danger: Color, val dark: Boolean,
)

private val Light = LpColors(
    board = Color(0xFFE6EAF0), paper = Color(0xFFFFFFFF), chrome = Color(0xFFF4F6F9), sunk = Color(0xFFEEF1F5),
    ink = Color(0xFF17202B), ink2 = Color(0xFF4B5767), ink3 = Color(0xFF596779), line = Color(0xFFC4CCD7), line2 = Color(0xFFE9ECF1),
    teal = Color(0xFF0D7266), tealInk = Color(0xFF0A5C52), tealSoft = Color(0xFFE2F1EE),
    attn = Color(0xFF8A5D00), attnSoft = Color(0xFFFFF4D4), attnLine = Color(0xFFE8C870),
    work = Color(0xFF1F63B0), workSoft = Color(0xFFE7F0FA), ok = Color(0xFF1A7F37), danger = Color(0xFFCF222E), dark = false,
)

private val Dark = LpColors(
    board = Color(0xFF090D12), paper = Color(0xFF0D1117), chrome = Color(0xFF161B22), sunk = Color(0xFF212830),
    ink = Color(0xFFE6EDF3), ink2 = Color(0xFFB6C2CF), ink3 = Color(0xFF98A6B7), line = Color(0xFF465363), line2 = Color(0xFF2B3542),
    teal = Color(0xFF0D7266), tealInk = Color(0xFF79D7C6), tealSoft = Color(0xFF14332E),
    attn = Color(0xFFEAC76A), attnSoft = Color(0xFF352B14), attnLine = Color(0xFF88702E),
    work = Color(0xFF83BAFF), workSoft = Color(0xFF172C46), ok = Color(0xFF70D68B), danger = Color(0xFFFF939A), dark = true,
)

val LocalLp = staticCompositionLocalOf { Light }

/** `MaterialTheme` with Launchpad's colours; `theme`: system, light or dark (This phone). */
@Composable
fun LaunchpadTheme(theme: String, content: @Composable () -> Unit) {
    val dark = when (theme) {
        "light" -> false
        "dark" -> true
        else -> isSystemInDarkTheme()
    }
    val c = if (dark) Dark else Light
    val scheme = if (dark) {
        darkColorScheme(
            primary = c.tealInk, onPrimary = Color(0xFF062A25), primaryContainer = c.tealSoft, onPrimaryContainer = c.tealInk,
            secondary = c.work, onSecondary = c.paper, secondaryContainer = c.workSoft, onSecondaryContainer = c.work,
            tertiary = c.attn, tertiaryContainer = c.attnSoft, onTertiaryContainer = c.attn,
            background = c.paper, onBackground = c.ink, surface = c.paper, onSurface = c.ink,
            surfaceVariant = c.sunk, onSurfaceVariant = c.ink2, outline = c.line, outlineVariant = c.line2,
            surfaceContainer = c.chrome, surfaceContainerHigh = c.chrome, surfaceContainerHighest = c.sunk, surfaceContainerLow = c.chrome, surfaceContainerLowest = c.board,
            error = c.danger, inverseSurface = c.ink, inverseOnSurface = c.paper,
        )
    } else {
        lightColorScheme(
            primary = c.teal, onPrimary = Color.White, primaryContainer = c.tealSoft, onPrimaryContainer = c.tealInk,
            secondary = c.work, onSecondary = Color.White, secondaryContainer = c.workSoft, onSecondaryContainer = c.work,
            tertiary = c.attn, tertiaryContainer = c.attnSoft, onTertiaryContainer = c.attn,
            background = c.paper, onBackground = c.ink, surface = c.paper, onSurface = c.ink,
            surfaceVariant = c.sunk, onSurfaceVariant = c.ink2, outline = c.line, outlineVariant = c.line2,
            surfaceContainer = c.chrome, surfaceContainerHigh = c.chrome, surfaceContainerHighest = c.sunk, surfaceContainerLow = c.chrome, surfaceContainerLowest = c.paper,
            error = c.danger, inverseSurface = c.ink, inverseOnSurface = c.paper,
        )
    }
    CompositionLocalProvider(LocalLp provides c) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
