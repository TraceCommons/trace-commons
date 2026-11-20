using System;
using System.Collections.Generic;
using System.Numerics;
using Microsoft.Graphics.Canvas;
using Microsoft.Graphics.Canvas.Geometry;
using Microsoft.Graphics.Canvas.Text;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop.Map;
using Windows.UI.ViewManagement;
using Color = Windows.UI.Color;
using TraceCommons.App.Look;

namespace TraceCommons.App.Controls;

/// <summary>
/// The tile in a tree row (<c>GlassToolTile</c>, Indicators.swift:247-334):
/// a tool's logo in its tint or its tinted initials, a folder, a folder's
/// first letter, an add box, or a session. Every value it draws is
/// <see cref="ToolTile.Face"/>'s; this control only paints it, with the
/// map's logo paths and the generated glass inks. Hidden from assistive
/// technology, as the Mac hides the tile: its row names it.
/// </summary>
internal sealed class ToolTileView : UserControl
{
    /// <summary>The Mac's SF Symbols as Segoe Fluent Icons code points.</summary>
    private static readonly IReadOnlyDictionary<string, string> Glyphs = new Dictionary<string, string>
    {
        [ToolTile.FolderGlyph] = "", // FolderFill
        [ToolTile.AddGlyph] = "", // Add
    };

    private readonly CanvasControl _canvas = new();
    private readonly AccessibilitySettings _accessibility = new();
    private readonly bool _large;
    private ToolTileKind _kind;

    public ToolTileView(ToolTileKind kind, bool large = false)
    {
        _kind = kind;
        _large = large;
        Width = Height = large ? ToolTile.SideLarge : ToolTile.Side;
        IsTabStop = false;
        _canvas.Draw += OnDraw;
        Content = _canvas;
        AutomationProperties.SetAccessibilityView(this, AccessibilityView.Raw);
        ActualThemeChanged += (_, _) => _canvas.Invalidate();
        // A live theme or high contrast switch too (ThemeBrushes).
        ThemeBrushes.RedrawOnChange(this, () => { _canvas.Invalidate(); });
        Unloaded += (_, _) => _canvas.RemoveFromVisualTree();
    }

    /// <summary>What the tile shows.</summary>
    public ToolTileKind Kind
    {
        get => _kind;
        set
        {
            if (Equals(_kind, value))
            {
                return;
            }

            _kind = value;
            _canvas.Invalidate();
        }
    }

    /// <summary>The core's folder mark (<c>glassFolderMark</c>), for a folder's letter when its name is empty.</summary>
    public string? FolderMark { get; set; }

    private Color Ink(string token, double alpha = 1, bool ground = false)
    {
        if (_accessibility.HighContrast)
        {
            return new UISettings().UIElementColor(ground ? UIElementType.Window : UIElementType.WindowText);
        }

        IReadOnlyDictionary<string, MapRgba> inks = ThemeBrushes.Dark ? MapArt.Dark : MapArt.Light;
        if (!inks.TryGetValue(token, out MapRgba c))
        {
            c = inks["textPrimary"];
        }

        return Color.FromArgb((byte)Math.Round(255 * Math.Clamp(c.A * alpha, 0, 1)), c.R, c.G, c.B);
    }

    private void OnDraw(CanvasControl sender, CanvasDrawEventArgs args)
    {
        ToolTileFace face = ToolTile.Face(_kind, _large, FolderMark);
        CanvasDrawingSession ds = args.DrawingSession;
        float side = (float)face.Side;
        float radius = (float)face.CornerRadius;
        ds.FillRoundedRectangle(0, 0, side, side, radius, radius, Ink(face.GroundToken, face.GroundAlpha, ground: true));

        Color ink = Ink(face.InkToken);
        var centre = new Vector2(side / 2, side / 2);
        switch (face.Mark)
        {
            case ToolTileMarkKind.Logo:
                if (MapArt.Logos.TryGetValue(face.Text, out LogoArtwork? artwork) && MapPainter.Logo(sender, face.Text) is { } logo)
                {
                    using (logo)
                    using (CanvasGeometry placed = logo.Transform(MapPainter.LogoFit(artwork, centre, face.MarkSize)))
                    {
                        ds.FillGeometry(placed, ink);
                    }
                }

                break;
            case ToolTileMarkKind.Text:
                DrawCentred(ds, face.Text, MapFontRole.Text, face, ink);
                break;
            case ToolTileMarkKind.Glyph:
                if (Glyphs.TryGetValue(face.Text, out string? glyph))
                {
                    DrawCentred(ds, glyph, MapFontRole.Icons, face, ink);
                }

                break;
        }
    }

    private static void DrawCentred(CanvasDrawingSession ds, string text, MapFontRole role, ToolTileFace face, Color ink)
    {
        using var format = new CanvasTextFormat
        {
            FontFamily = MapFonts.Family(role, CanvasTextFormat.GetSystemFontFamilies()),
            FontSize = (float)face.MarkSize,
            FontWeight = face.Weight switch
            {
                ToolTileWeight.Bold => Microsoft.UI.Text.FontWeights.Bold,
                ToolTileWeight.Semibold => Microsoft.UI.Text.FontWeights.SemiBold,
                _ => Microsoft.UI.Text.FontWeights.Normal,
            },
            HorizontalAlignment = CanvasHorizontalAlignment.Center,
            VerticalAlignment = CanvasVerticalAlignment.Center,
            WordWrapping = CanvasWordWrapping.NoWrap,
        };
        ds.DrawText(text, new Windows.Foundation.Rect(0, 0, face.Side, face.Side), ink, format);
    }
}
