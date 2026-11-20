using System;
using System.Runtime.InteropServices;
using Microsoft.Graphics.Canvas;
using TraceCommons.Interop.FirstRun;

namespace TraceCommons.App.FirstRun;

/// <summary>
/// Whether a Win2D device draws with software rendering: its DXGI adapter
/// is the Microsoft Basic Render Driver (WARP) or flagged software
/// (<see cref="StageBackdropBudget.IsSoftwareAdapter"/>). The adapter is
/// the device's own, read through its DXGI interface, not a guess at the
/// system's first adapter. Any failure reads as hardware: the stage's frame
/// rate still catches a machine that cannot keep up.
/// </summary>
internal static class RenderAdapter
{
    private static readonly Guid DxgiInterfaceAccess = new("A9B3D012-3DF2-4EE3-B8D1-8695F457D3C1");
    private static readonly Guid DxgiDevice = new("54ec77fa-1377-44e6-8c32-88fd5f44c84c");
    private static readonly Guid DxgiAdapter1 = new("29038f61-3839-4626-91fd-086879011a05");

    // IDirect3DDxgiInterfaceAccess::GetInterface, after IUnknown's three.
    private const int GetInterfaceSlot = 3;

    // IDXGIDevice::GetAdapter, after IUnknown's three and IDXGIObject's four.
    private const int GetAdapterSlot = 7;

    // IDXGIAdapter1::GetDesc1, after IDXGIAdapter's EnumOutputs, GetDesc, CheckInterfaceSupport.
    private const int GetDesc1Slot = 10;

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int GetInterfaceFn(IntPtr self, ref Guid iid, out IntPtr result);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int GetAdapterFn(IntPtr self, out IntPtr adapter);

    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int GetDesc1Fn(IntPtr self, IntPtr desc);

    /// <summary>What the adapter reported, for the look log: vendor, device, flags; null when it could not be read.</summary>
    public static (uint Vendor, uint Device, uint Flags)? Read(CanvasDevice device)
    {
        IntPtr unknown = IntPtr.Zero, access = IntPtr.Zero, dxgi = IntPtr.Zero, adapter = IntPtr.Zero, adapter1 = IntPtr.Zero, desc = IntPtr.Zero;
        try
        {
            unknown = WinRT.MarshalInspectable<object>.FromManaged(device);
            Guid iid = DxgiInterfaceAccess;
            if (unknown == IntPtr.Zero || Marshal.QueryInterface(unknown, ref iid, out access) < 0)
            {
                return null;
            }

            iid = DxgiDevice;
            if (Slot<GetInterfaceFn>(access, GetInterfaceSlot)(access, ref iid, out dxgi) < 0
                || Slot<GetAdapterFn>(dxgi, GetAdapterSlot)(dxgi, out adapter) < 0)
            {
                return null;
            }

            iid = DxgiAdapter1;
            if (Marshal.QueryInterface(adapter, ref iid, out adapter1) < 0)
            {
                return null;
            }

            // DXGI_ADAPTER_DESC1: Description[128] (WCHAR), VendorId, DeviceId,
            // SubSysId, Revision, three SIZE_T, an 8-byte LUID, Flags.
            const int Vendor = 256, DeviceId = 260, AfterRevision = 272;
            int flagsAt = AfterRevision + (3 * IntPtr.Size) + 8;
            desc = Marshal.AllocHGlobal(flagsAt + 8);
            if (Slot<GetDesc1Fn>(adapter1, GetDesc1Slot)(adapter1, desc) < 0)
            {
                return null;
            }

            return ((uint)Marshal.ReadInt32(desc, Vendor), (uint)Marshal.ReadInt32(desc, DeviceId), (uint)Marshal.ReadInt32(desc, flagsAt));
        }
        catch (Exception e) when (e is COMException or InvalidCastException or MarshalDirectiveException or ObjectDisposedException)
        {
            return null;
        }
        finally
        {
            if (desc != IntPtr.Zero)
            {
                Marshal.FreeHGlobal(desc);
            }

            foreach (IntPtr held in new[] { adapter1, adapter, dxgi, access, unknown })
            {
                if (held != IntPtr.Zero)
                {
                    Marshal.Release(held);
                }
            }
        }
    }

    /// <summary>Whether the adapter <see cref="Read"/> returned draws in software; false when it could not be read.</summary>
    public static bool IsSoftware((uint Vendor, uint Device, uint Flags)? adapter) =>
        adapter is { } a && StageBackdropBudget.IsSoftwareAdapter(a.Vendor, a.Device, a.Flags);

    private static T Slot<T>(IntPtr self, int slot)
        where T : Delegate =>
        Marshal.GetDelegateForFunctionPointer<T>(Marshal.ReadIntPtr(Marshal.ReadIntPtr(self), slot * IntPtr.Size));
}
