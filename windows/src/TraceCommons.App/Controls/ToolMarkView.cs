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
/// A tool's mark as the transfer card's coin draws it
/// (<c>TraceTransferCard.coin</c>, TraceTransferCard.swift:116-128): an
/// opaque face, a 2-point rim in the tool's tint, and its logo 14 across or
/// its initials, monospaced 10 bold, in the tint. A batch from several
/// tools has no tool's mark: the stock copy glyph, as Linux draws for the
/// Mac's <c>square.stack</c> (transfer_card.rs). Drawn with the map's
/// own logo paths and inks. Decoration only: it is named by its card.
/// </summary>
internal sealed class ToolMarkView : UserControl
{
    /// <summary><c>GlassNoticeMark.side</c>.</summary>
    public const double Side = 30;

    private readonly CanvasControl _canvas = new();
    private readonly FontIcon _mixed = new() { Glyph = "\uE8C8", FontSize = 16, Visibility = Visibility.Collapsed };
    private readonly AccessibilitySettings _accessibility = new();
    private GlassToolArt? _art;

    public ToolMarkView()
    {
        Width = Height = Side;
        IsTabStop = false;
        _canvas.Draw += OnDraw;
        var grid = new Grid();
        grid.Children.Add(_canvas);
        grid.Children.Add(_mixed);
        Content = grid;
        AutomationProperties.SetAccessibilityView(this, AccessibilityView.Raw);
        ActualThemeChanged += (_, _) => _canvas.Invalidate();
        // A live theme or high contrast switch too (ThemeBrushes).
        ThemeBrushes.RedrawOnChange(this, () => { _canvas.Invalidate(); });
        Unloaded += (_, _) => _canvas.RemoveFromVisualTree();
    }

    /// <summary>The tool to draw; null for a batch from several tools.</summary>
    public GlassToolArt? Art
    {
        get => _art;
        set
        {
            _art = value;
            _mixed.Visibility = value is null ? Visibility.Visible : Visibility.Collapsed;
            _canvas.Visibility = value is null ? Visibility.Collapsed : Visibility.Visible;
            _canvas.Invalidate();
        }
    }

    private Color Ink(string token)
    {
        if (_accessibility.HighContrast)
        {
            return new UISettings().UIElementColor(token == "paneOpaque" ? UIElementType.Window : UIElementType.WindowText);
        }

        IReadOnlyDictionary<string, MapRgba> inks = ThemeBrushes.Dark ? MapArt.Dark : MapArt.Light;
        if (!inks.TryGetValue(token, out MapRgba c))
        {
            c = inks["textPrimary"];
        }

        return Color.FromArgb((byte)Math.Round(255 * Math.Clamp(c.A, 0, 1)), c.R, c.G, c.B);
    }

    private void OnDraw(CanvasControl sender, CanvasDrawEventArgs args)
    {
        if (_art is not { } art)
        {
            return;
        }

        CanvasDrawingSession ds = args.DrawingSession;
        var centre = new Vector2((float)(Side / 2), (float)(Side / 2));
        Color tint = Ink(art.TintToken);
        ds.FillCircle(centre, (float)(Side / 2), Ink("paneOpaque"));
        // strokeBorder: the 2-point rim lies inside the edge.
        ds.DrawCircle(centre, (float)(Side / 2 - 1), tint, 2);
        if (art.Logo is { } name && MapArt.Logos.TryGetValue(name, out LogoArtwork? artwork)
            && MapPainter.Logo(sender, name) is { } logo)
        {
            using (logo)
            using (CanvasGeometry placed = logo.Transform(MapPainter.LogoFit(artwork, centre, 14)))
            {
                ds.FillGeometry(placed, tint);
            }

            return;
        }

        using var format = new CanvasTextFormat
        {
            FontFamily = MapFonts.Family(MapFontRole.Mono, CanvasTextFormat.GetSystemFontFamilies()),
            FontSize = 10,
            FontWeight = Microsoft.UI.Text.FontWeights.Bold,
            HorizontalAlignment = CanvasHorizontalAlignment.Center,
            VerticalAlignment = CanvasVerticalAlignment.Center,
        };
        ds.DrawText(art.Initials, new Windows.Foundation.Rect(0, 0, Side, Side), tint, format);
    }
}
