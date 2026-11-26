using System.Collections.Generic;

namespace TraceCommons.Interop.Shell;

public static class FlowRows
{
    /// <summary>Use the preferred split only when it fits in two rows and the unsplit row does not.</summary>
    public static int PreferredBreak(IReadOnlyList<double> widths, double availableWidth, double spacing, int after)
    {
        if (after <= 0 || after >= widths.Count)
        {
            return 0;
        }

        double first = (after - 1) * spacing;
        for (int i = 0; i < after; i++)
        {
            first += widths[i];
        }

        double second = (widths.Count - after - 1) * spacing;
        for (int i = after; i < widths.Count; i++)
        {
            second += widths[i];
        }

        return first <= availableWidth && second <= availableWidth && first + spacing + second > availableWidth ? after : 0;
    }
}
