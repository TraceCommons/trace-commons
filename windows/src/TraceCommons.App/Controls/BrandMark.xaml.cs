using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace TraceCommons.App.Controls;

/// <summary>
/// The Trace Commons mark, "The Turn".
///
/// Two properties: how big to draw it, and whether to draw the framed variant
/// or the frameless single-ink template one. Everything else about the mark is
/// fixed by the design spec and belongs in the XAML, not in an API.
///
/// The mark is vector geometry on a 64-unit grid scaled by a Viewbox, so the
/// sizes the spec calls for -- 14, 15, 16, 20, 22, 40 and 84 px -- are all the
/// same drawing at different scales rather than separate assets. At the small
/// end the frame is a 2-unit stroke, which is half a device pixel at 16 px:
/// thin, which is what a hairline frame is, and it is the frame rather than
/// the brackets that thins, so the mark still reads as two corners.
/// </summary>
public sealed partial class BrandMark : UserControl
{
    /// <summary>
    /// The rendered edge length in effective pixels. The mark is square, so
    /// one number sets both dimensions.
    /// </summary>
    public static readonly DependencyProperty SizeProperty =
        DependencyProperty.Register(
            nameof(Size),
            typeof(double),
            typeof(BrandMark),
            new PropertyMetadata(16.0, OnSizeChanged));

    /// <summary>
    /// Draws the frameless, single-ink variant instead of the framed one.
    /// Intended for a tray icon or a menu surface -- anywhere the host
    /// supplies its own background and expects to recolour what it is given.
    /// </summary>
    public static readonly DependencyProperty IsTemplateVariantProperty =
        DependencyProperty.Register(
            nameof(IsTemplateVariant),
            typeof(bool),
            typeof(BrandMark),
            new PropertyMetadata(false, OnIsTemplateVariantChanged));

    public BrandMark()
    {
        InitializeComponent();

        // The property callbacks do not fire for a default value, so the
        // initial state is applied once here.
        ApplySize();
        ApplyVariant();
    }

    public double Size
    {
        get => (double)GetValue(SizeProperty);
        set => SetValue(SizeProperty, value);
    }

    public bool IsTemplateVariant
    {
        get => (bool)GetValue(IsTemplateVariantProperty);
        set => SetValue(IsTemplateVariantProperty, value);
    }

    private static void OnSizeChanged(DependencyObject sender, DependencyPropertyChangedEventArgs args) =>
        ((BrandMark)sender).ApplySize();

    private static void OnIsTemplateVariantChanged(DependencyObject sender, DependencyPropertyChangedEventArgs args) =>
        ((BrandMark)sender).ApplyVariant();

    /// <summary>
    /// Sizes the control itself rather than the Viewbox inside it, so that
    /// layout around the mark reserves the right space whether or not the
    /// mark has been measured yet.
    /// </summary>
    private void ApplySize()
    {
        Width = Size;
        Height = Size;
    }

    private void ApplyVariant()
    {
        FramedMark.Visibility = IsTemplateVariant ? Visibility.Collapsed : Visibility.Visible;
        TemplateMark.Visibility = IsTemplateVariant ? Visibility.Visible : Visibility.Collapsed;
    }
}
