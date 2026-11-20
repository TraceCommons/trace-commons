using System;
using System.Collections.Generic;

namespace TraceCommons.Interop.Map;

/// <summary>A value animating from one number to another on a curve, from a start time.</summary>
public sealed class AnimatedValue
{
    private double _from;
    private double _to;
    private double _start;
    private MotionCurve? _curve;

    public AnimatedValue(double value)
    {
        _from = _to = value;
    }

    /// <summary>The value it is heading for.</summary>
    public double Target => _to;

    /// <summary>The value at <paramref name="now"/>.</summary>
    public double At(double now) => _curve is null ? _to : _from + (_to - _from) * _curve.At(now - _start);

    /// <summary>Whether it is still moving at <paramref name="now"/>.</summary>
    public bool Moving(double now) => _curve is not null && now - _start < _curve.Duration;

    /// <summary>
    /// Heads for <paramref name="to"/> from wherever it is at
    /// <paramref name="now"/>, as SwiftUI retargets an animation; null
    /// <paramref name="curve"/> jumps there (reduce motion).
    /// </summary>
    public void Set(double to, double now, MotionCurve? curve)
    {
        if (to == _to)
        {
            return;
        }

        _from = At(now);
        _to = to;
        _start = now;
        _curve = curve;
    }
}

/// <summary>
/// The tool pile's motion (MapToolPile.swift:17-200), on an explicit clock:
/// the card's tall, wide and names stages under the pointer, each coin's
/// place in the pile, its opening and its growing as a trace arrives.
/// </summary>
public sealed class ToolPileState
{
    private readonly AnimatedValue _tall = new(0);
    private readonly AnimatedValue _wide = new(0);
    private readonly AnimatedValue _names = new(0);
    private readonly Dictionary<string, AnimatedValue> _place = new(StringComparer.Ordinal);
    private readonly Dictionary<string, AnimatedValue> _open = new(StringComparer.Ordinal);
    private readonly Dictionary<string, AnimatedValue> _grown = new(StringComparer.Ordinal);
    private bool _inside;
    private double _hoverAt;

    /// <summary>The pointer entered or left the card (:173-199).</summary>
    public void Hover(bool inside, double now)
    {
        if (inside == _inside)
        {
            return;
        }

        _inside = inside;
        _hoverAt = now;
    }

    /// <summary>The stages at <paramref name="now"/>, each 0 (closed) to 1 (open).</summary>
    public (double Tall, double Wide, double Names) Stages(double now, bool reduceMotion)
    {
        foreach (PileStage stage in _inside ? MapMotion.PileOpen : MapMotion.PileClose)
        {
            AnimatedValue value = stage.Part switch
            {
                PilePart.Tall => _tall,
                PilePart.Wide => _wide,
                _ => _names,
            };
            if (now - _hoverAt >= stage.Delay)
            {
                value.Set(_inside ? 1 : 0, _hoverAt + stage.Delay, reduceMotion ? null : stage.Curve);
            }
        }

        return (_tall.At(now), _wide.At(now), _names.At(now));
    }

    /// <summary>Whether the pile is open: tall and nothing moving (:47).</summary>
    public bool Open(bool moving) => _tall.Target > 0 && !moving;

    /// <summary>
    /// Each coin's index (front first), its opening (0 closed, 1 open) and
    /// its grown scale at <paramref name="now"/>, retargeted from
    /// <paramref name="order"/>, <paramref name="open"/> and
    /// <paramref name="grown"/> (:126-131).
    /// </summary>
    public (double Index, double Open, double Scale) Coin(string id, int index, bool open, bool grown, double now, bool reduceMotion)
    {
        AnimatedValue place = Get(_place, id, index);
        AnimatedValue opening = Get(_open, id, open ? 1 : 0);
        AnimatedValue scale = Get(_grown, id, grown ? MapMotion.CoinGrown : 1);
        place.Set(index, now, reduceMotion ? null : MapMotion.CoinShuffle);
        opening.Set(open ? 1 : 0, now, reduceMotion ? null : MapMotion.CoinOpen);
        scale.Set(grown ? MapMotion.CoinGrown : 1, now, reduceMotion ? null : MapMotion.CoinGrow);
        return (place.At(now), opening.At(now), scale.At(now));
    }

    /// <summary>Whether any of the pile's motion is still running at <paramref name="now"/>.</summary>
    public bool Moving(double now)
    {
        if (_tall.Moving(now) || _wide.Moving(now) || _names.Moving(now))
        {
            return true;
        }

        // A stage still waiting for its delay is motion to come.
        foreach (PileStage stage in _inside ? MapMotion.PileOpen : MapMotion.PileClose)
        {
            if (now - _hoverAt < stage.Delay + stage.Curve.Duration)
            {
                return true;
            }
        }

        foreach (Dictionary<string, AnimatedValue> map in new[] { _place, _open, _grown })
        {
            foreach (AnimatedValue value in map.Values)
            {
                if (value.Moving(now))
                {
                    return true;
                }
            }
        }

        return false;
    }

    private static AnimatedValue Get(Dictionary<string, AnimatedValue> map, string id, double initial)
    {
        if (!map.TryGetValue(id, out AnimatedValue? value))
        {
            map[id] = value = new AnimatedValue(initial);
        }

        return value;
    }
}
