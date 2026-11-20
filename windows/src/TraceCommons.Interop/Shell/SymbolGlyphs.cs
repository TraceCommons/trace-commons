using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The one table of SF Symbols the Mac app draws and the Segoe icon glyph
/// Windows draws for each (spec W5-D2 (a)): the Mac's own glyph paths are
/// exported (<see cref="GlassGlyphs"/>); the SF Symbols, which are Apple's
/// only, map to a code point of Segoe Fluent Icons, falling back to Segoe
/// MDL2 Assets, which carries the same code points for these glyphs on
/// Windows 10 (<see cref="FontFamily"/>).
/// </summary>
/// <remarks>
/// <para>
/// The names are every literal <c>systemName:</c> and <c>systemImage:</c> in
/// macos/Sources outside the Gallery at the base (35), and the symbols the Mac
/// names through a constant: <c>arrow.clockwise</c>
/// (GlassRefreshButton.systemImage), <c>chevron.down</c> and <c>chevron.up</c>,
/// <c>exclamationmark.triangle.fill</c>, <c>magnifyingglass</c> and
/// <c>line.3.horizontal.decrease</c>.
/// </para>
/// <para>
/// A glyph marked <see cref="SymbolGlyph.Nearest"/> has no like-for-like
/// Segoe glyph; it is the nearest in meaning and is listed for the owner.
/// </para>
/// </remarks>
public static class SymbolGlyphs
{
    /// <summary>The icon font, with the Windows 10 font as its fallback.</summary>
    public const string FontFamily = "Segoe Fluent Icons, Segoe MDL2 Assets";

    public static readonly IReadOnlyDictionary<string, SymbolGlyph> Table = new Dictionary<string, SymbolGlyph>
    {
        ["arrow.clockwise"] = new(0xE72C),
        ["arrow.right"] = new(0xE72A),
        ["bell"] = new(0xEA8F),
        ["binoculars"] = new(0xE721, Nearest: true),
        ["checklist"] = new(0xE9D5),
        ["checkmark"] = new(0xE73E),
        ["chevron.down"] = new(0xE70D),
        ["chevron.left"] = new(0xE76B),
        ["chevron.right"] = new(0xE76C),
        ["chevron.up"] = new(0xE70E),
        ["circle.fill"] = new(0xEA3B),
        ["clock"] = new(0xE823),
        ["creditcard"] = new(0xE8C7),
        ["doc.text"] = new(0xE8A5),
        ["dollarsign.circle"] = new(0xE8C7, Nearest: true),
        ["exclamationmark.circle.fill"] = new(0xE783),
        ["exclamationmark.triangle.fill"] = new(0xE7BA),
        // FolderFill, filled as the symbol; the outline Folder (E8B7) was
        // the launch art's largest pixel difference from the Mac (#2207).
        ["folder.fill"] = new(0xE8D5),
        ["gearshape"] = new(0xE713),
        ["hand.thumbsup"] = new(0xE8E1),
        ["hand.thumbsup.fill"] = new(0xE8E1),
        ["info.circle"] = new(0xE946),
        ["lightbulb"] = new(0xEA80),
        ["line.3.horizontal.decrease"] = new(0xE71C),
        ["lock"] = new(0xE72E),
        ["lock.fill"] = new(0xE72E),
        ["lock.shield"] = new(0xEA18, Nearest: true),
        ["magnifyingglass"] = new(0xE721),
        ["minus"] = new(0xE738),
        ["nosign"] = new(0xE733),
        ["person.crop.circle.badge.checkmark"] = new(0xE77B, Nearest: true),
        ["plus"] = new(0xE710),
        ["power"] = new(0xE7E8),
        ["shield.lefthalf.filled"] = new(0xEA18),
        ["sparkles"] = new(0xE734, Nearest: true),
        ["star.fill"] = new(0xE735),
        ["terminal"] = new(0xE756),
        ["testtube.2"] = new(0xE9F9, Nearest: true),
        ["tray"] = new(0xE896, Nearest: true),
        ["tray.and.arrow.down"] = new(0xE896),
        ["xmark"] = new(0xE711),
    };

    /// <summary>The glyph for an SF Symbol name, or null when the table has none.</summary>
    public static SymbolGlyph? For(string symbol) => Table.TryGetValue(symbol, out SymbolGlyph glyph) ? glyph : null;
}

/// <summary>A Segoe icon code point; <paramref name="Nearest"/> when it is not like for like.</summary>
public readonly record struct SymbolGlyph(int CodePoint, bool Nearest = false)
{
    /// <summary>The glyph as the one-character string a FontIcon takes.</summary>
    public string Text => char.ConvertFromUtf32(CodePoint);
}
