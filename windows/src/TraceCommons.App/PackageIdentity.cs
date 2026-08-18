using System.Runtime.InteropServices;

namespace TraceCommons.App;

/// <summary>
/// Whether this process is running with MSIX package identity.
/// </summary>
/// <remarks>
/// <para>
/// The app ships unpackaged by default and can be built packaged
/// (<c>TcPackaged=true</c>, see <c>windows/packaging/README.md</c>). The two
/// flavours are not interchangeable for anything that writes to the registry
/// on the user's behalf: the package manifest disables registry
/// virtualization, so a write that is harmlessly redirected in some packaged
/// apps is REAL here, and the path it would record points inside
/// <c>WindowsApps</c>, which the package model does not want anyone launching
/// directly.
/// </para>
/// <para>
/// P/Invoke rather than <c>Windows.ApplicationModel.Package.Current</c>: that
/// property THROWS when there is no package, so using it would mean an
/// exception on every startup of the build actually shipped.
/// <c>GetCurrentPackageFullName</c> returns <c>APPMODEL_ERROR_NO_PACKAGE</c>
/// instead, which is an answer rather than an incident.
/// </para>
/// <para>
/// <c>UrlSchemeRegistration</c> carries its own private copy of this check,
/// added with the packaging support. The two should be collapsed onto this
/// class; that edit is left to whoever owns that file next rather than made
/// from here.
/// </para>
/// </remarks>
internal static class PackageIdentity
{
    private const int AppModelErrorNoPackage = 15700;

    /// <summary>True when a package identity exists for this process.</summary>
    internal static bool IsPackaged()
    {
        int length = 0;
        return GetCurrentPackageFullName(ref length, null) != AppModelErrorNoPackage;
    }

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern int GetCurrentPackageFullName(ref int packageFullNameLength, char[]? packageFullName);
}
