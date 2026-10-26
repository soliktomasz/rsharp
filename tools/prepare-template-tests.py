#!/usr/bin/env python3
"""Prepare the live template tests' local feed; never install into the person's template hive."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import urllib.request

root = Path(sys.argv[1] if len(sys.argv) > 1 else "target/template-tests").resolve()
root.mkdir(parents=True, exist_ok=True)
feed = root / "feed"
feed.mkdir(exist_ok=True)
version = "12.1.4"  # Passenger's Directory.Packages.props, AvaloniaTemplatesVersion.
package = f"avalonia.templates.{version}.nupkg"
if not (feed / package).exists():
    urllib.request.urlretrieve(f"https://api.nuget.org/v3-flatcontainer/avalonia.templates/{version}/{package}", feed / package)
environment = dict(os.environ, DOTNET_CLI_HOME=str(root / "home"), NUGET_PACKAGES=str(root / "packages"), DOTNET_NOLOGO="1", DOTNET_CLI_TELEMETRY_OPTOUT="1")
project = root / "prepare"
if project.exists():
    shutil.rmtree(project)
subprocess.run(["dotnet", "new", "xunit", "--name", "Prerequisites", "--output", str(project), "--no-restore", "--debug:custom-hive", str(root / "hive")], env=environment, check=True)
path = project / "Prerequisites.csproj"
text = path.read_text(encoding="utf-8-sig")
references = "".join(f'<PackageReference Include="{name}" Version="{version}" />' for name in ["Avalonia", "Avalonia.Desktop", "Avalonia.Themes.Fluent", "Avalonia.Fonts.Inter"])
path.write_text(text.replace("</Project>", f"<ItemGroup>{references}</ItemGroup></Project>"), encoding="utf-8")
config = project / "nuget.config"
config.write_text('<configuration><packageSources><clear/><add key="nuget" value="https://api.nuget.org/v3/index.json" /></packageSources></configuration>', encoding="utf-8")
subprocess.run(["dotnet", "restore", str(path), "--configfile", str(config)], env=environment, check=True)
# Passenger's preview SDK also builds net10.0 executables: cache their reference packs and native app host.
subprocess.run(["dotnet", "restore", str(path), "--configfile", str(config), "-p:TargetFramework=net10.0", "-p:OutputType=Exe", "-p:UseAppHost=true"], env=environment, check=True)
for archive in (root / "packages").rglob("*.nupkg"):
    destination = feed / archive.name
    if not destination.exists():
        shutil.copy2(archive, destination)
print(f"Live template feed ready: {feed}")
