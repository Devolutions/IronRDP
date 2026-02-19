using System.Diagnostics;
using WixSharp;

using CompressionLevel = WixSharp.CompressionLevel;
using File = WixSharp.File;

namespace IronRdpCevicheInstaller;

internal static class Program
{
    private const string PackageName = "IronRdpCevicheService";

    private static string ServiceExecutablePath => ResolveRequiredArtifact(
        "IRDP_CEVICHE_SERVICE_EXECUTABLE",
        "..\\..\\target\\release\\ceviche-service.exe");

    private static string? ProviderDllPath => ResolveOptionalArtifact(
        "IRDP_PROVIDER_DLL",
        "..\\..\\target\\release\\ironrdp_wtsprotocol_provider.dll");

    private static string? ConfigDirectoryPath => ResolveOptionalDirectory("IRDP_CEVICHE_CONFIG_DIR");

    private static Version InstallerVersion
    {
        get
        {
            var versionString = Environment.GetEnvironmentVariable("IRDP_CEVICHE_MSI_VERSION");

            if (string.IsNullOrWhiteSpace(versionString))
            {
                versionString = FileVersionInfo.GetVersionInfo(ServiceExecutablePath).FileVersion;
            }

            if (string.IsNullOrWhiteSpace(versionString) || !Version.TryParse(versionString, out var version))
            {
                throw new Exception("IRDP_CEVICHE_MSI_VERSION is not specified or invalid");
            }

            return NormalizeInstallerVersion(version);
        }
    }

    private static Platform TargetPlatform
    {
        get
        {
            var platform = Environment.GetEnvironmentVariable("IRDP_CEVICHE_MSI_PLATFORM");

#if DEBUG
            if (string.IsNullOrWhiteSpace(platform))
            {
                return Platform.x64;
            }
#endif

            if (string.IsNullOrWhiteSpace(platform))
            {
                throw new Exception("IRDP_CEVICHE_MSI_PLATFORM is not specified");
            }

            return platform.ToLowerInvariant() switch
            {
                "x64" or "x86_64" or "amd64" => Platform.x64,
                "x86" or "i386" => Platform.x86,
                "arm64" or "aarch64" => Platform.arm64,
                _ => throw new Exception($"unsupported IRDP_CEVICHE_MSI_PLATFORM value: {platform}"),
            };
        }
    }

    private static string ResolveRequiredArtifact(string varName, string defaultPath)
    {
        var path = ResolveOptionalArtifact(varName, defaultPath);
        if (path is null)
        {
            throw new Exception($"required artifact is missing ({varName})");
        }

        return path;
    }

    private static string? ResolveOptionalArtifact(string varName, string? defaultPath = null)
    {
        var configuredPath = Environment.GetEnvironmentVariable(varName);
        if (!string.IsNullOrWhiteSpace(configuredPath))
        {
            var fullPath = Path.GetFullPath(configuredPath);
            if (!System.IO.File.Exists(fullPath))
            {
                throw new FileNotFoundException($"artifact from {varName} does not exist", fullPath);
            }

            return fullPath;
        }

        if (string.IsNullOrWhiteSpace(defaultPath))
        {
            return null;
        }

        var candidate = Path.GetFullPath(defaultPath);
        if (!System.IO.File.Exists(candidate))
        {
#if DEBUG
            return null;
#else
            throw new FileNotFoundException($"default artifact path does not exist for {varName}", candidate);
#endif
        }

        return candidate;
    }

    private static string? ResolveOptionalDirectory(string varName)
    {
        var configured = Environment.GetEnvironmentVariable(varName);
        if (string.IsNullOrWhiteSpace(configured))
        {
            return null;
        }

        var fullPath = Path.GetFullPath(configured);
        if (!Directory.Exists(fullPath))
        {
            throw new DirectoryNotFoundException($"directory from {varName} does not exist: {fullPath}");
        }

        return fullPath;
    }

    private static Version NormalizeInstallerVersion(Version input)
    {
        var major = ClampToRange(input.Major, 0, 255);
        var minor = ClampToRange(input.Minor, 0, 255);
        var build = ClampToRange(input.Build < 0 ? 0 : input.Build, 0, 65535);

        return new Version(major, minor, build);
    }

    private static int ClampToRange(int value, int minimum, int maximum)
    {
        if (value < minimum)
        {
            return minimum;
        }

        if (value > maximum)
        {
            return maximum;
        }

        return value;
    }

    private static void Main()
    {
        var payload = new List<WixEntity>();

        var serviceFile = new File(ServiceExecutablePath)
        {
            TargetFileName = Includes.SERVICE_EXECUTABLE_NAME,
            ServiceInstaller = new ServiceInstaller
            {
                Type = SvcType.ownProcess,
                Interactive = false,
                Vital = true,
                Name = Includes.SERVICE_NAME,
                DisplayName = Includes.SERVICE_DISPLAY_NAME,
                Description = Includes.SERVICE_DESCRIPTION,
                FirstFailureActionType = FailureActionType.restart,
                SecondFailureActionType = FailureActionType.restart,
                ThirdFailureActionType = FailureActionType.restart,
                RestartServiceDelayInSeconds = 60,
                ResetPeriodInDays = 1,
                RemoveOn = SvcEvent.Uninstall,
                StopOn = SvcEvent.InstallUninstall,
            },
        };

        payload.Add(serviceFile);

        if (ProviderDllPath is { } providerDllPath)
        {
            payload.Add(new File(providerDllPath)
            {
                TargetFileName = Includes.PROVIDER_DLL_NAME,
            });
        }

        if (ConfigDirectoryPath is { } configDirectoryPath)
        {
            payload.Add(new Dir("config", new Files($"{configDirectoryPath}\\*.*")));
        }

        var project = new ManagedProject(Includes.PRODUCT_NAME)
        {
            UpgradeCode = Includes.UPGRADE_CODE,
            Version = InstallerVersion,
            Description = Includes.SERVICE_DESCRIPTION,
            InstallerVersion = 500,
            InstallScope = InstallScope.perMachine,
            InstallPrivileges = InstallPrivileges.elevated,
            Platform = TargetPlatform,
#if DEBUG
            PreserveTempFiles = true,
            OutDir = "Debug",
#else
            OutDir = "Release",
#endif
            OutFileName = PackageName,
            MajorUpgrade = new MajorUpgrade
            {
                AllowDowngrades = false,
                AllowSameVersionUpgrades = true,
                DowngradeErrorMessage = "A newer version is already installed.",
                Schedule = UpgradeSchedule.afterInstallInitialize,
                MigrateFeatures = true,
            },
            Media = new List<Media>
            {
                new()
                {
                    Cabinet = "ceviche.cab",
                    EmbedCab = true,
                    CompressionLevel = CompressionLevel.mszip,
                },
            },
            ControlPanelInfo = new ProductInfo
            {
                Manufacturer = Includes.VENDOR_NAME,
                NoModify = true,
                UrlInfoAbout = Includes.INFO_URL,
            },
            Dirs = new[]
            {
                new Dir("%ProgramFiles%", new Dir(Includes.VENDOR_NAME, new InstallDir(Includes.SHORT_NAME, payload.ToArray()))),
            },
            RegValues = new[]
            {
                new RegValue(
                    RegistryHive.LocalMachine,
                    $"Software\\{Includes.VENDOR_NAME}\\{Includes.SHORT_NAME}",
                    "InstallDir",
                    "[INSTALLDIR]")
                {
                    AttributesDefinition = "Type=string; Component:Permanent=yes",
                    Win64 = TargetPlatform is Platform.x64 or Platform.arm64,
                    RegistryKeyAction = RegistryKeyAction.create,
                },
                new RegValue(
                    RegistryHive.LocalMachine,
                    $"Software\\{Includes.VENDOR_NAME}\\{Includes.SHORT_NAME}",
                    "ServicePath",
                    $"[INSTALLDIR]{Includes.SERVICE_EXECUTABLE_NAME}")
                {
                    AttributesDefinition = "Type=string",
                    Win64 = TargetPlatform is Platform.x64 or Platform.arm64,
                    RegistryKeyAction = RegistryKeyAction.createAndRemoveOnUninstall,
                },
            },
        };

        var msiPath = project.BuildMsi();
        Console.WriteLine($"MSI built: {msiPath}");
    }
}
