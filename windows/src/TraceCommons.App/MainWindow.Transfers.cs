using System;
using System.Collections.Generic;
using System.Linq;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.App.Controls;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// The transfer cards (<c>TraceTransferCard</c>; W4, approved as a class
/// 2026-11-19): in front of every notice on Traces and History
/// (MonitorNotices.swift:157-161), and at the map's foot above the legend
/// while another tab is shown, stacked as the Mac's peeking stack from the
/// bottom (FlowMapView.swift:166-187, W4-D1 (a)).
/// </summary>
public sealed partial class MainWindow
{
    /// <summary>The notice stack's transfer card: one element, showing whichever transfer is in front.</summary>
    private readonly TransferCardView _transferNotice = new(closeStyle: null) { Visibility = Visibility.Collapsed };

    private TransferCardView? _mapTransferCard;
    private readonly Border[] _mapTransferPlates = new Border[2];
    private string? _mapTransferFront;
    private IReadOnlySet<string> _mapTransfersSeen = new HashSet<string>(StringComparer.Ordinal);

    private void InitializeTransfers(TracesMapView map)
    {
        // The stack's X stays over every card: the card goes in under it.
        NoticeFront.Children.Insert(NoticeFront.Children.IndexOf(NoticeClose), _transferNotice);

        _mapTransferCard = new TransferCardView((Style)WindowRoot.Resources["TcToolbarButtonStyle"]) { VerticalAlignment = VerticalAlignment.Bottom };
        _mapTransferCard.CloseRequested += () =>
        {
            if (_mapTransferCard.TransferId is { } id)
            {
                map.CloseTransfer(id);
            }
        };
        _mapTransferCard.SizeChanged += (_, _) => LayoutMapTransferPlates();
        for (int i = _mapTransferPlates.Length - 1; i >= 0; i--)
        {
            var plate = new Border
            {
                VerticalAlignment = VerticalAlignment.Bottom,
                IsHitTestVisible = false,
                BorderThickness = new Thickness(1),
                CornerRadius = (CornerRadius)Application.Current.Resources["TcCardCornerRadius"],
                Visibility = Visibility.Collapsed,
            };
            _mapTransferPlates[i] = plate;
            map.TransferFoot.Children.Add(plate);
        }

        map.TransferFoot.Children.Add(_mapTransferCard);
        map.TransferFoot.Visibility = Visibility.Collapsed;
        map.TransfersChanged += () =>
        {
            RefreshNotices();
            RefreshMapTransfers();
        };
    }

    /// <summary>Each card the player holds, oldest first, as a notice of Traces and History.</summary>
    private IEnumerable<(NoticeKind Kind, FrameworkElement Card)> TransferNotices() =>
        _map is null
            ? Enumerable.Empty<(NoticeKind, FrameworkElement)>()
            : _map.TransferCards.Select(t => (NoticeCatalog.Transfer(t.Id), (FrameworkElement)_transferNotice)).ToArray();

    /// <summary>Fills the notice stack's transfer card when a transfer is in front.</summary>
    private void ShowTransferNotice(NoticeKind front)
    {
        if (_map is null || !NoticeCatalog.IsTransfer(front.Id) || FlowMapCopy.Table is not { } words)
        {
            return;
        }

        string id = NoticeCatalog.TransferId(front.Id);
        if (_map.TransferCards.FirstOrDefault(t => t.Id == id) is { } transfer)
        {
            _transferNotice.Show(id, TransferCardFace.For(transfer, words, _map.FolderLabel(transfer.Folder), null));
        }
    }

    /// <summary>The map's foot: the cards while another tab than Traces is shown.</summary>
    private void RefreshMapTransfers()
    {
        if (_map is null || _mapTransferCard is null)
        {
            return;
        }

        IReadOnlyList<MapTransfer> cards = _map.TransferCards;
        if (!TransferCardFace.OnMap(_at.Tab == MonitorTab.Traces, _map.ShowsTraces, cards.Count) || FlowMapCopy.Table is not { } words)
        {
            _map.TransferFoot.Visibility = Visibility.Collapsed;
            _mapTransferFront = null;
            return;
        }

        MapTransfer front = cards[^1];
        _mapTransferCard.Show(front.Id, TransferCardFace.For(front, words, _map.FolderLabel(front.Folder), null));
        _map.TransferFoot.Visibility = Visibility.Visible;
        string[] ids = cards.Select(c => c.Id).ToArray();
        if (_mapTransferFront != front.Id)
        {
            // A card new to the foot rises from the bottom and fades in, on
            // the stack's spring (PopoverNotice.swift:238, anchor .bottom).
            ResetMotion(_mapTransferCard);
            NoticeChange change = NoticeMotion.Plan(
                _mapTransferFront, front.Id, _mapTransfersSeen, _uiSettings.AnimationsEnabled && _clockGate.MayTick);
            if (change.Entrance == NoticeEntrance.Slide)
            {
                double height = _mapTransferCard.ActualHeight;
                Animate(_mapTransferCard, "Translation.Y", t => NoticeMotion.ArrivalOffset(StackAnchor.Bottom, height, t), null);
                Animate(_mapTransferCard, "Opacity", NoticeMotion.FadeIn, null);
            }

            _mapTransferFront = front.Id;
        }

        _mapTransfersSeen = NoticeStack.SeenAfter(_mapTransfersSeen, ids);
        LayoutMapTransferPlates();
    }

    /// <summary>The plates behind the map's front card, its size, peeking above it.</summary>
    private void LayoutMapTransferPlates()
    {
        if (_map is null || _mapTransferCard is null)
        {
            return;
        }

        string[] ids = _map.TransferCards.Select(c => c.Id).ToArray();
        IReadOnlyList<NoticePlate> plates = NoticeStack.Plates(ids, StackAnchor.Bottom);
        double width = _mapTransferCard.ActualWidth;
        double height = _mapTransferCard.ActualHeight;
        for (int i = 0; i < _mapTransferPlates.Length; i++)
        {
            Border plate = _mapTransferPlates[i];
            if (i >= plates.Count || height <= 0)
            {
                plate.Visibility = Visibility.Collapsed;
                continue;
            }

            NoticePlate p = plates[i];
            plate.Background = (Brush)Application.Current.Resources["TcSurfaceCardBrush"];
            plate.BorderBrush = (Brush)Application.Current.Resources["TcHairlineBrush"];
            plate.Height = height;
            plate.Opacity = p.Opacity;
            plate.RenderTransform = new CompositeTransform { ScaleX = p.ScaleX, CenterX = width / 2, TranslateY = p.OffsetY };
            plate.Visibility = Visibility.Visible;
        }

        _map.TransferFoot.Padding = new Thickness(0, NoticeStack.PlateRoom(ids), 0, 0);
    }
}
