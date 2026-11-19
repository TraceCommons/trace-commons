using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Every C# binding of the contributor's C ABI names a function the header
/// declares, with a matching signature, and the loaded library exports it.
/// </summary>
/// <remarks>
/// <para>
/// Slice W1 of <c>docs/superpowers/specs/2026-11-18-windows-mac-parity-design.md</c>
/// (one-pass checklist I1). The Mac's Swift is compiled against
/// <c>trace_commons.h</c>, so a wrong declaration there fails its build. C#
/// declares each function again by hand in <c>NativeMethods.cs</c>, and
/// until now nothing compared the two: a renamed export, a dropped parameter
/// or a 32-bit field read as 64-bit only showed up when that call ran.
/// </para>
/// <para>
/// The bindings are read by reflection from the built assembly, not from the
/// source text. The header is the contract the Rust is checked against
/// (<c>trace-commons-contributor-ffi</c>'s own header test), so it is read as
/// the expected side.
/// </para>
/// </remarks>
public class NativeBindingTests
{
    private const string Library = "trace_commons_contributor_ffi";

    /// <summary>
    /// How many C ABI functions C# binds. Moves only on purpose: a slice that
    /// binds a new export states the old and new numbers (checklist I1).
    /// </summary>
    private const int BoundFunctions = 130;

    /// <summary>How many functions the header declares (both copies are identical).</summary>
    private const int HeaderFunctions = 187;

    private static string HeaderPath => Path.Combine(AppContext.BaseDirectory, "abi", "trace_commons.h.txt");

    [Fact]
    public void TheHeaderParsesToEveryDeclaredFunction()
    {
        IReadOnlyDictionary<string, CFunction> header = ParseHeader(File.ReadAllText(HeaderPath));
        Assert.Equal(HeaderFunctions, header.Count);

        // Two shapes the parser must get right, pinned to the header's own
        // declarations.
        Assert.Equal(new[] { "ptr", "ptr", "ptrptr" }, header["tc_daemon_start_with_settings"].Parameters);
        Assert.Equal("ptr", header["tc_daemon_start_with_settings"].Return);
        Assert.Equal(new[] { "ptr", "fn", "ptr" }, header["tc_subscribe"].Parameters);
        Assert.Equal("u64", header["tc_subscribe"].Return);
    }

    [Fact]
    public void EveryBindingNamesAHeaderFunctionWithTheSameSignature()
    {
        IReadOnlyDictionary<string, CFunction> header = ParseHeader(File.ReadAllText(HeaderPath));
        IReadOnlyList<Binding> bindings = Bindings();

        var problems = new List<string>();
        foreach (Binding binding in bindings)
        {
            if (!header.TryGetValue(binding.EntryPoint, out CFunction? c))
            {
                problems.Add($"{binding.EntryPoint}: not in trace_commons.h");
                continue;
            }

            if (binding.Return != c.Return)
            {
                problems.Add($"{binding.EntryPoint}: returns {binding.Return} in C#, {c.Return} in the header");
            }

            if (!binding.Parameters.SequenceEqual(c.Parameters))
            {
                problems.Add(
                    $"{binding.EntryPoint}: takes ({string.Join(", ", binding.Parameters)}) in C#, "
                    + $"({string.Join(", ", c.Parameters)}) in the header");
            }

            if (binding.CallingConvention != CallingConvention.Cdecl)
            {
                problems.Add($"{binding.EntryPoint}: calling convention {binding.CallingConvention}, not Cdecl");
            }
        }

        Assert.True(problems.Count == 0, string.Join("\n", problems));
        Assert.Equal(BoundFunctions, bindings.Count);
        Assert.Equal(bindings.Count, bindings.Select(b => b.EntryPoint).Distinct().Count());
    }

    /// <summary>
    /// The library this run loaded exports every function the header
    /// declares. A stale library (checklist I2) fails here, by name, instead
    /// of agreeing with an equally stale binding list.
    /// </summary>
    [Fact]
    public void TheLoadedLibraryExportsEveryHeaderFunction()
    {
        IReadOnlyDictionary<string, CFunction> header = ParseHeader(File.ReadAllText(HeaderPath));
        IntPtr library = NativeLibrary.Load(Library, typeof(TcDaemon).Assembly, null);
        string[] missing = header.Keys
            .Where(name => !NativeLibrary.TryGetExport(library, name, out _))
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToArray();
        Assert.True(missing.Length == 0, "not exported: " + string.Join(", ", missing));
    }

    private sealed record CFunction(string Name, string Return, string[] Parameters);

    private sealed record Binding(string EntryPoint, string Return, string[] Parameters, CallingConvention CallingConvention);

    /// <summary>
    /// Every <c>DllImport</c> of the cdylib in the Interop assembly, as the
    /// runtime sees it.
    /// </summary>
    private static IReadOnlyList<Binding> Bindings()
    {
        var bindings = new List<Binding>();
        foreach (Type type in typeof(TcDaemon).Assembly.GetTypes())
        {
            foreach (MethodInfo method in type.GetMethods(
                BindingFlags.Static | BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.DeclaredOnly))
            {
                DllImportAttribute? import = method.GetCustomAttribute<DllImportAttribute>();
                if (import is null || import.Value != Library)
                {
                    continue;
                }

                bindings.Add(new Binding(
                    import.EntryPoint ?? method.Name,
                    ManagedKind(method.ReturnParameter),
                    method.GetParameters().Select(ManagedKind).ToArray(),
                    import.CallingConvention));
            }
        }

        return bindings;
    }

    /// <summary>A managed parameter as the C kind it must marshal to.</summary>
    private static string ManagedKind(ParameterInfo parameter)
    {
        Type type = parameter.ParameterType;
        if (type.IsByRef)
        {
            // out/ref IntPtr is the address of a pointer: char** in C.
            return type.GetElementType() == typeof(IntPtr) ? "ptrptr" : "byref " + type.GetElementType()!.Name;
        }

        if (typeof(Delegate).IsAssignableFrom(type))
        {
            return "fn";
        }

        if (type == typeof(void))
        {
            return "void";
        }

        if (type == typeof(IntPtr) || type == typeof(string) || type == typeof(byte[]) || type.IsPointer)
        {
            return "ptr";
        }

        if (type == typeof(UIntPtr))
        {
            return "usize";
        }

        if (type == typeof(bool))
        {
            // A C bool is one byte; the default marshalling of bool is four.
            MarshalAsAttribute? marshal = parameter.GetCustomAttribute<MarshalAsAttribute>();
            return marshal?.Value is UnmanagedType.U1 or UnmanagedType.I1 ? "u8" : "bool4";
        }

        return Type.GetTypeCode(type) switch
        {
            TypeCode.Byte => "u8",
            TypeCode.Int32 => "i32",
            TypeCode.UInt32 => "u32",
            TypeCode.Int64 => "i64",
            TypeCode.UInt64 => "u64",
            TypeCode.Double => "f64",
            _ => "managed " + type.Name,
        };
    }

    /// <summary>
    /// The header's function declarations, by name. Comments and
    /// preprocessor lines are dropped, and a declaration is a <c>tc_</c> name
    /// followed by a balanced parameter list and a semicolon.
    /// </summary>
    private static IReadOnlyDictionary<string, CFunction> ParseHeader(string text)
    {
        text = Regex.Replace(text, @"/\*.*?\*/", " ", RegexOptions.Singleline);
        text = Regex.Replace(text, @"//[^\n]*", " ");
        text = string.Join("\n", text.Split('\n').Where(line => !line.TrimStart().StartsWith('#')));

        var functions = new Dictionary<string, CFunction>(StringComparer.Ordinal);
        foreach (Match match in Regex.Matches(text, @"\b(tc_\w+)\s*\("))
        {
            int end = match.Index + match.Length;
            int depth = 1;
            while (depth > 0)
            {
                depth += text[end] switch { '(' => 1, ')' => -1, _ => 0 };
                end++;
            }

            if (!text[end..].TrimStart().StartsWith(';'))
            {
                continue;
            }

            int start = new[] { ';', '}', '{' }.Max(c => text.LastIndexOf(c, match.Index)) + 1;
            string returnType = text[start..match.Index];
            string parameters = text[(match.Index + match.Length)..(end - 1)];
            string name = match.Groups[1].Value;
            Assert.False(functions.ContainsKey(name), $"{name} is declared twice");
            functions[name] = new CFunction(name, CKind(returnType), SplitParameters(parameters).Select(CKind).ToArray());
        }

        return functions;
    }

    private static IEnumerable<string> SplitParameters(string parameters)
    {
        var current = new System.Text.StringBuilder();
        int depth = 0;
        foreach (char c in parameters)
        {
            if (c == ',' && depth == 0)
            {
                yield return current.ToString();
                current.Clear();
                continue;
            }

            depth += c switch { '(' => 1, ')' => -1, _ => 0 };
            current.Append(c);
        }

        string last = current.ToString().Trim();
        if (last.Length > 0 && last != "void")
        {
            yield return last;
        }
    }

    /// <summary>A C parameter or return type as a marshalling kind.</summary>
    private static string CKind(string declaration)
    {
        string text = declaration.Trim();
        if (text.Contains('('))
        {
            return "fn";
        }

        int stars = text.Count(c => c == '*');
        if (stars >= 2)
        {
            return "ptrptr";
        }

        if (stars == 1)
        {
            return "ptr";
        }

        string[] words = text.Split(new[] { ' ', '\t', '\n', '\r' }, StringSplitOptions.RemoveEmptyEntries)
            .Where(word => word is not ("const" or "extern" or "static" or "inline"))
            .ToArray();
        Assert.True(words.Length > 0, $"no type in '{declaration}'");
        return words[0] switch
        {
            "void" => "void",
            "bool" or "uint8_t" => "u8",
            "int32_t" or "int" => "i32",
            "uint32_t" => "u32",
            "int64_t" => "i64",
            "uint64_t" => "u64",
            "size_t" or "uintptr_t" => "usize",
            "double" => "f64",
            _ => "c " + words[0],
        };
    }
}
