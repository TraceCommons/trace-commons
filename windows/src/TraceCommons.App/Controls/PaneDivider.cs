using System;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;

namespace TraceCommons.App.Controls;

/// <summary>The gap grip shared by the main pane and the map.</summary>
public sealed class PaneDivider : Grid
{
    private Pointer? _pointer;
    private InputSystemCursor? _resizeCursor;
    private double _startX;
    private bool _dragging;
    private bool _hovered;

    public PaneDivider()
    {
        PointerEntered += OnEntered;
        PointerExited += (_, _) => SetHovered(false);
        PointerPressed += OnPressed;
        PointerMoved += OnMoved;
        PointerReleased += OnReleased;
        PointerCanceled += OnCanceled;
        PointerCaptureLost += OnCanceled;
    }

    public event Action<double>? Dragged;
    public event Action<bool>? DragEnded;

    public void CancelDrag()
    {
        SetHovered(false);
        EndDrag(commit: false);
    }

    private void OnEntered(object sender, PointerRoutedEventArgs args)
    {
        // WinUI requires the XAML tree to finish initializing before assigning a cursor.
        if (args.Pointer.PointerDeviceType == PointerDeviceType.Mouse && _resizeCursor is null)
        {
            _resizeCursor = InputSystemCursor.Create(InputSystemCursorShape.SizeWestEast);
            ProtectedCursor = _resizeCursor;
        }

        SetHovered(true);
    }

    private void OnPressed(object sender, PointerRoutedEventArgs args)
    {
        var point = args.GetCurrentPoint(null);
        if (_pointer is not null
            || (args.Pointer.PointerDeviceType == PointerDeviceType.Mouse && !point.Properties.IsLeftButtonPressed)
            || !CapturePointer(args.Pointer))
        {
            return;
        }

        _pointer = args.Pointer;
        // Measure in window coordinates so the moving grip cannot feed back into its own movement.
        _startX = point.Position.X;
        args.Handled = true;
    }

    private void OnMoved(object sender, PointerRoutedEventArgs args)
    {
        if (_pointer?.PointerId != args.Pointer.PointerId)
        {
            return;
        }

        double dx = args.GetCurrentPoint(null).Position.X - _startX;
        if (_dragging || dx != 0)
        {
            _dragging = true;
            UpdateGrip();
            Dragged?.Invoke(dx);
        }

        args.Handled = true;
    }

    private void OnReleased(object sender, PointerRoutedEventArgs args)
    {
        if (_pointer?.PointerId != args.Pointer.PointerId)
        {
            return;
        }

        if (_dragging)
        {
            Dragged?.Invoke(args.GetCurrentPoint(null).Position.X - _startX);
        }

        EndDrag(commit: true);
        args.Handled = true;
    }

    private void OnCanceled(object sender, PointerRoutedEventArgs args)
    {
        if (_pointer?.PointerId == args.Pointer.PointerId)
        {
            CancelDrag();
        }
    }

    private void EndDrag(bool commit)
    {
        Pointer? pointer = _pointer;
        bool dragging = _dragging;
        _pointer = null;
        _dragging = false;
        if (pointer is not null)
        {
            ReleasePointerCapture(pointer);
        }

        UpdateGrip();
        if (dragging)
        {
            DragEnded?.Invoke(commit);
        }
    }

    private void SetHovered(bool hovered)
    {
        _hovered = hovered;
        UpdateGrip();
    }

    private void UpdateGrip()
    {
        if (Children.Count != 0)
        {
            Children[0].Opacity = _hovered || _dragging ? 0.6 : 0;
        }
    }
}
