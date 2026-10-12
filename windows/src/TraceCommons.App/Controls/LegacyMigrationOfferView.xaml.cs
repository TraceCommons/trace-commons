using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;

namespace TraceCommons.App.Controls;

/// <summary>
/// The legacy invite migration offer, drawn from
/// <see cref="LegacyMigrationOfferViewModel"/>. Thin by design: every word
/// and every decision is in the view model and below it.
/// </summary>
public sealed partial class LegacyMigrationOfferView : UserControl
{
    public static readonly DependencyProperty OfferProperty = DependencyProperty.Register(
        nameof(Offer),
        typeof(LegacyMigrationOfferViewModel),
        typeof(LegacyMigrationOfferView),
        new PropertyMetadata(null, (d, _) => ((LegacyMigrationOfferView)d).Bindings?.Update()));

    public static readonly DependencyProperty ShowsDismissProperty = DependencyProperty.Register(
        nameof(ShowsDismiss),
        typeof(bool),
        typeof(LegacyMigrationOfferView),
        new PropertyMetadata(false, (d, _) => ((LegacyMigrationOfferView)d).Bindings?.Update()));

    public LegacyMigrationOfferView()
    {
        InitializeComponent();
    }

    /// <summary>Never null once drawn: the hosts assign it before showing the control.</summary>
    public LegacyMigrationOfferViewModel Offer
    {
        get => (LegacyMigrationOfferViewModel)GetValue(OfferProperty);
        set => SetValue(OfferProperty, value);
    }

    /// <summary>The notices card can be put away for good; the Settings section cannot.</summary>
    public bool ShowsDismiss
    {
        get => (bool)GetValue(ShowsDismissProperty);
        set => SetValue(ShowsDismissProperty, value);
    }

    private async void OnMove(object sender, RoutedEventArgs e) => await Offer.MoveAsync();

    private async void OnCancel(object sender, RoutedEventArgs e) => await Offer.CancelAsync();

    private void OnDismiss(object sender, RoutedEventArgs e) => Offer.DismissCard();
}
