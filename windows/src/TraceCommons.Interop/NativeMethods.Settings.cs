using System;
using System.Runtime.InteropServices;

namespace TraceCommons.Interop;

/// <summary>
/// The C ABI the Settings window's sections read (Windows parity spec W16,
/// W17 and W18), declared beside the rest of <see cref="NativeMethods"/> so
/// the Settings slices add their bindings in one place. Each is checked
/// against <c>trace_commons.h</c> by <c>NativeBindingTests</c>.
/// </summary>
internal static partial class NativeMethods
{
    /// <summary>The General section's words (<c>settings_general_copy</c>). Owned JSON.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_settings_general_copy_json();

    /// <summary>The Devices section's words (<c>consent_copy::account_devices_copy</c>). Owned JSON.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_account_devices_copy_json();

    /// <summary>The payout-link section's words (<c>consent_copy::payout_link_copy</c>). Owned JSON.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_payout_link_copy_json();

    /// <summary>The referral words (<c>referral_copy::referral_copy</c>). Owned JSON.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_referral_copy_json();

    /// <summary>A typed referral code as it is shown, or NULL for text that is not a code.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_referral_code_display([MarshalAs(UnmanagedType.LPUTF8Str)] string typed);

    /// <summary>The account section's own Set up again (<c>consent_copy::set_up_again_copy</c>). Owned JSON.</summary>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_set_up_again_copy_json();

    // ---- Compute (W18). One handle per owner; free it only once the
    // controller says its worker stopped and no command is pending.

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_compute_open([MarshalAs(UnmanagedType.LPUTF8Str)] string configDir, out IntPtr err);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_compute_status_json(IntPtr handle);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_compute_resource_begin_json(IntPtr handle);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_compute_resource_event_json(IntPtr handle, [MarshalAs(UnmanagedType.LPUTF8Str)] string json);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_compute_command_json(IntPtr handle, [MarshalAs(UnmanagedType.LPUTF8Str)] string commandJson);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern void tc_compute_free(IntPtr handle);

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_compute_copy_json();

    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern IntPtr tc_compute_shutdown(IntPtr handle, ulong timeoutMs);
}
