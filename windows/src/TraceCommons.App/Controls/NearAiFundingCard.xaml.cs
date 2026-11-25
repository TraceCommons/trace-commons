using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

public sealed partial class NearAiFundingCard : UserControl
{
    public NearAiFundingCard(NearAiFunding funding)
    {
        Funding = funding;
        InitializeComponent();
        Loaded += async (_, _) => await Funding.ActivateAsync();
        Unloaded += (_, _) => Funding.Deactivate();
    }

    public NearAiFunding Funding { get; }

    private async void OnFundingAction(object sender, RoutedEventArgs e) => await Funding.PressAsync();
}
