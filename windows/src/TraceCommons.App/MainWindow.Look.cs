using System;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Numerics;
using Microsoft.UI.Composition.SystemBackdrops;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Media;
using TraceCommons.App.Look;
using TraceCommons.Interop.Shell;
using Windows.UI.ViewManagement;

namespace TraceCommons.App;

/// <summary>
/// The window's look (spec W5): the material under its panes and the tab
/// ember. Kept apart from the shell's layout so the look lands without moving
/// anything else.
/// </summary>
public sealed partial class MainWindow
{
    private WindowMaterial _material = WindowMaterial.Painted;
    private bool _windowActive = true;
    private (double X, double Width)? _emberAt;

    // ---- The material (W5-D1 (b)) -------------------------------------------

    /// <summary>
    /// Mica where the system reports it supported and no high contrast theme
    /// is on; the painted flat glass otherwise. Over Mica the window's ground
    /// is the pane's veil, which reads at 4.5:1 over a black or a white
    /// desktop (GlassTokensTests). Unfocused, the ground takes its inactive
    /// twin, as the Mac dims an inactive window's panes. The choice and the
    /// support reading are logged (checklist D4).
    /// </summary>
    private void ApplyGlassMaterial()
    {
        bool supported = MicaController.IsSupported();
        bool highContrast = new Windows.UI.ViewManagement.AccessibilitySettings().HighContrast;
        _material = WindowMaterialChoice.For(supported, highContrast);
        LookLog($"material: mica supported={supported} highContrast={highContrast} chosen={_material}");
        if (_material == WindowMaterial.Mica)
        {
            SystemBackdrop = new MicaBackdrop();
        }

        Activated += (_, args) =>
        {
            _windowActive = args.WindowActivationState != WindowActivationState.Deactivated;
            PaintGround();
            // A window brought forward reads the system's motion setting
            // again, in case the system raised no change.
            GlassMotionSettings.Refresh();
        };
        WindowRoot.ActualThemeChanged += (_, _) => PaintGround();
        WindowRoot.Loaded += (_, _) => PaintGround();
        // A strip laid out again (a tab's count, a scale change) puts the
        // ember under its tab at once; only a change of tab glides.
        TabStrip.SizeChanged += (_, _) =>
        {
            _emberAt = null;
            MoveTabEmber();
        };
    }

    /// <summary>The window's ground and title bar in the key for the material and focus.</summary>
    private void PaintGround()
    {
        string key = WindowMaterialChoice.GroundKey(_material, _windowActive);
        if (LookBrush(key) is Brush ground)
        {
            WindowRoot.Background = ground;
            AppTitleBar.Background = ground;
        }
    }

    /// <summary>A generated brush in the window's current theme.</summary>
    private Brush? LookBrush(string key)
    {
        string theme = WindowRoot.ActualTheme == ElementTheme.Light ? "Light" : "Default";
        if (new AccessibilitySettings().HighContrast)
        {
            theme = "HighContrast";
        }

        foreach (ResourceDictionary dictionary in Application.Current.Resources.MergedDictionaries.SelectMany(Flatten))
        {
            if (dictionary.ThemeDictionaries.TryGetValue(theme, out object? themed)
                && themed is ResourceDictionary scheme
                && scheme.TryGetValue(key, out object? value)
                && value is Brush brush)
            {
                return brush;
            }
        }

        return null;
    }

    private static System.Collections.Generic.IEnumerable<ResourceDictionary> Flatten(ResourceDictionary dictionary)
    {
        yield return dictionary;
        foreach (ResourceDictionary merged in dictionary.MergedDictionaries.SelectMany(Flatten))
        {
            yield return merged;
        }
    }

    /// <summary>
    /// Writes a line about the look to Debug output and, when
    /// TRACE_COMMONS_LOOK_LOG names a file, to that file, so the test box can
    /// read which material a run chose. Never a secret or a path of the
    /// person's.
    /// </summary>
    private static void LookLog(string line)
    {
        Debug.WriteLine("monitor: " + line);
        // "1" writes to tc-look.log in the temporary folder, so the test
        // box can turn it on through win-capture.ps1, whose values carry no
        // path characters.
        string? path = Environment.GetEnvironmentVariable("TRACE_COMMONS_LOOK_LOG");
        if (path == "1")
        {
            path = Path.Combine(Path.GetTempPath(), "tc-look.log");
        }

        if (!string.IsNullOrEmpty(path))
        {
            try
            {
                File.AppendAllText(path, $"{DateTime.UtcNow:o} {line}{Environment.NewLine}");
            }
            catch (IOException)
            {
            }
            catch (UnauthorizedAccessException)
            {
            }
        }
    }

    // ---- The tab ember (Navigation.swift:266-339) --------------------------

    /// <summary>
    /// Puts the ember under the tab shown. It takes the new tab's frame at
    /// once and its visual glides there from the old tab's on the Mac's
    /// spring(0.35, 0.85) (TabEmberGlide), unless things may not move.
    /// </summary>
    private void MoveTabEmber()
    {
        ToggleButton? shown = TabStrip.Children.OfType<ToggleButton>().FirstOrDefault(b => b.IsChecked == true);
        if (shown is null || shown.ActualWidth <= 0)
        {
            TabEmber.Visibility = Visibility.Collapsed;
            if (shown is not null)
            {
                shown.SizeChanged -= OnShownTabSized;
                shown.SizeChanged += OnShownTabSized;
            }

            return;
        }

        double x = shown.TransformToVisual(TabEmberLayer).TransformPoint(new Windows.Foundation.Point(0, 0)).X;
        double width = shown.ActualWidth;
        (double emberX, double emberWidth) = GlassTabMotion.Ember(x, width);
        TabEmber.Visibility = Visibility.Visible;
        TabEmber.Width = emberWidth;
        TabEmberHalo.Width = GlassTabMotion.Halo(x, width).Width;
        Canvas.SetLeft(TabEmber, emberX);

        ElementCompositionPreview.SetIsTranslationEnabled(TabEmber, true);
        var visual = ElementCompositionPreview.GetElementVisual(TabEmber);
        visual.CenterPoint = new Vector3((float)(emberWidth / 2), (float)(TabEmber.Height / 2), 0);

        if (_emberAt is (double oldX, double oldWidth) && (oldX != x || oldWidth != width))
        {
            (double translate, double scale) = TabEmberGlide.From(oldX, oldWidth, x, width);
            GlassComposition.Run(TabEmber, "Translation.X", (float)translate, 0, GlassControlMotion.TabEmberGlide);
            GlassComposition.Run(TabEmber, "Scale.X", (float)scale, 1, GlassControlMotion.TabEmberGlide);
        }
        else
        {
            GlassComposition.Settle(TabEmber, "Translation.X", 0);
            GlassComposition.Settle(TabEmber, "Scale.X", 1);
        }

        _emberAt = (x, width);
    }

    private void OnShownTabSized(object sender, SizeChangedEventArgs e)
    {
        ((FrameworkElement)sender).SizeChanged -= OnShownTabSized;
        _emberAt = null;
        MoveTabEmber();
    }
}
