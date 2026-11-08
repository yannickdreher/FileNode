"""Export the canonical SVG identity to desktop and website assets."""
from pathlib import Path
import shutil
import subprocess

root = Path(__file__).resolve().parents[1]
branding = root / "assets" / "branding"
windows = root / "apps" / "windows" / "Assets"
renderer = shutil.which("magick") or shutil.which("convert")
svg_renderer = shutil.which("rsvg-convert")
if not renderer or not svg_renderer:
    raise SystemExit("Install ImageMagick and librsvg (rsvg-convert) to regenerate branding assets.")


def render(source: Path, destination: Path, size: str) -> None:
    width, height = size.split("x")
    subprocess.run([svg_renderer, "--width", width, "--height", height,
                    "--output", str(destination), str(source)], check=True)


for target in (root / "apps/linux/drive.svg", root / "website/mark.svg"):
    shutil.copyfile(branding / "icon.svg", target)
for filename, pixels in (("icon.png", 512), ("Square44x44Logo.png", 44),
                         ("Square150x150Logo.png", 150), ("StoreLogo.png", 50)):
    render(branding / "icon.svg", windows / filename, f"{pixels}x{pixels}")
shutil.copyfile(windows / "icon.png", branding / "icon.png")
render(branding / "logo.svg", branding / "logo.png", "1160x240")
subprocess.run([renderer, str(windows / "icon.png"), "-define",
                "icon:auto-resize=256,128,64,48,40,32,24,20,16",
                str(windows / "drive.ico")], check=True)
print("Exported NoriDrive SVG, PNG, ICO and Windows package graphics.")
