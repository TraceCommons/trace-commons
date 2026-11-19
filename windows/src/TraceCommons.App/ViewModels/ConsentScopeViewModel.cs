using System;
using System.ComponentModel;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>A scope row: the core's title and description.</summary>
public sealed class ConsentScopeViewModel : INotifyPropertyChanged
{
    private bool _isSelected;

    public ConsentScopeViewModel(ConsentOption scope)
    {
        ArgumentNullException.ThrowIfNull(scope);

        Name = scope.Name;
        Description = scope.Description;
        AlwaysOn = scope.AlwaysOn;
        Title = scope.Title ?? string.Empty;
        _isSelected = scope.AlwaysOn;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string Name { get; }

    /// <summary>The daemon's own words, verbatim.</summary>
    public string Description { get; }

    public string Title { get; }

    public bool AlwaysOn { get; }

    /// <summary>An always-on scope is shown checked and cannot be unchecked.</summary>
    public bool IsEnabled => !AlwaysOn;

    public bool IsSelected
    {
        get => _isSelected;
        set
        {
            if (_isSelected == value || AlwaysOn)
            {
                return;
            }

            _isSelected = value;
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(nameof(IsSelected)));
        }
    }
}
