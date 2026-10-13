"""Renders docs/figures/*.png from the data dumped by `examples/figures.rs`.

Usage: python3 tools/plot_figures.py <data-dir> <out-dir>
Needs numpy, matplotlib and pillow.
"""
import csv, sys
import numpy as np
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.colors import LinearSegmentedColormap, ListedColormap
from PIL import Image

D = sys.argv[1]
OUT = sys.argv[2]
W, H, DAYS = map(int, open(f"{D}/meta.txt").read().split())

SURFACE, INK, INK2, MUTED, GRID, AXIS = "#fcfcfb", "#0b0b0b", "#52514e", "#898781", "#e1e0d9", "#c3c2b7"
BLUE, RED = "#2a78d6", "#e34948"
DIVERGING = LinearSegmentedColormap.from_list(
    "div", ["#104281", BLUE, "#86b6ef", "#f0efec", "#f2b8b5", RED, "#9e2a2b"])
plt.rcParams.update({
    "figure.facecolor": SURFACE, "axes.facecolor": SURFACE, "savefig.facecolor": SURFACE,
    "text.color": INK, "axes.labelcolor": INK2, "xtick.color": MUTED, "ytick.color": MUTED,
    "axes.edgecolor": AXIS, "axes.grid": True, "grid.color": GRID, "grid.linewidth": 0.6,
    "axes.spines.top": False, "axes.spines.right": False, "font.size": 10,
    "axes.titlesize": 11, "axes.titleweight": "bold", "axes.titlelocation": "left",
})

def grid(name):
    return np.fromfile(f"{D}/{name}", dtype="<i4").reshape(H, W)

water = grid("water.bin")
land = water == 1

def coastline(ax):
    ax.contour(land.astype(float), levels=[0.5], colors=INK2, linewidths=0.6)

def lat_axis(ax):
    ys = [(90 - lat) / 180 * H - 0.5 for lat in (60, 30, 0, -30, -60)]
    ax.set_yticks(ys, ["60°N", "30°N", "0°", "30°S", "60°S"])
    ax.set_xticks([])
    ax.grid(False)
    for s in ax.spines.values():
        s.set_visible(False)

def anomaly_map(ax, field, limit, title, label="°C"):
    im = ax.imshow(field, cmap=DIVERGING, vmin=-limit, vmax=limit, interpolation="nearest")
    coastline(ax)
    lat_axis(ax)
    ax.set_title(title)
    cb = plt.colorbar(im, ax=ax, fraction=0.025, pad=0.02)
    cb.set_label(label, color=INK2)
    cb.outline.set_visible(False)
    cb.ax.tick_params(colors=MUTED)

daily = list(csv.DictReader(open(f"{D}/daily.csv")))
day = np.array([int(r["day"]) for r in daily])
years = day / DAYS + 1
raw_index = np.array([int(r["index_tenths"]) for r in daily]) / 10
# The index is stored in 0.1 °C steps; a centered 30-day mean shows the smooth signal.
index = np.convolve(raw_index, np.ones(30) / 30, mode="same")
drift = np.array([int(r["drift_tenths"]) for r in daily]) / 10
gmean = np.array([int(r["global_mean_tenths"]) for r in daily]) / 10
seaice = np.array([float(r["sea_ice_pct"]) for r in daily])

def rolling_year(v):
    k = np.ones(DAYS) / DAYS
    out = np.convolve(v, k, mode="valid")
    return years[DAYS - 1:], out

def date_of(d):
    y, doy = d // DAYS + 1, d % DAYS
    season = ["spring", "summer", "autumn", "winter"][doy * 4 // DAYS]
    return f"year {y}, northern {season}"

# ---------------------------------------------------------------- ENSO: time series
fig, axes = plt.subplots(3, 1, figsize=(12, 8.2), sharex=True,
                         gridspec_kw={"height_ratios": [2.2, 1, 1]})
ax = axes[0]
ax.axhline(0, color=AXIS, lw=1)
for t in (0.5, -0.5):
    ax.axhline(t, color=MUTED, lw=0.8, ls=(0, (4, 3)))
ax.fill_between(years, index, 0.5, where=index >= 0.5, color=RED, alpha=0.85, lw=0, interpolate=True)
ax.fill_between(years, index, -0.5, where=index <= -0.5, color=BLUE, alpha=0.85, lw=0, interpolate=True)
ax.plot(years, index, color=INK2, lw=1.2)
ax.set_title("ENSO-like oscillation index over 50 years (30-day mean)")
ax.set_ylabel("warm-pool anomaly, °C")
ax.text(1.2, 2.1, "warm phase (El Niño-like) ≥ +0.5 °C", color=RED, fontsize=9, weight="bold")
ax.text(1.2, -2.6, "cold phase (La Niña-like) ≤ −0.5 °C", color=BLUE, fontsize=9, weight="bold")
warm = np.argmax(index); cold = np.argmin(index)
for i, col, va, off in ((warm, RED, "bottom", 0.15), (cold, BLUE, "top", -0.15)):
    ax.plot(years[i], index[i], "o", ms=7, color=col, mec=SURFACE, mew=2)
    ax.annotate(f"strongest: {index[i]:+.1f} °C", (years[i], index[i] + off), ha="center",
                va=va, color=INK2, fontsize=9)
ax.set_ylim(-3.2, 2.8)

ax = axes[1]
x, y = rolling_year(gmean)
ax.plot(x, y, color=BLUE, lw=2)
ax.set_title("Global mean temperature, 1-year running mean")
ax.set_ylabel("°C")
ax = axes[2]
x, y = rolling_year(seaice)
ax.plot(x, y, color=BLUE, lw=2)
ax.set_title("Sea ice, % of ocean area, 1-year running mean")
ax.set_ylabel("%")
ax.set_xlabel("simulated year")
ax.set_xlim(1, years[-1])
fig.tight_layout()
fig.savefig(f"{OUT}/1-enso-index.png", dpi=110)
plt.close(fig)

# ---------------------------------------------------------------- ENSO: maps
weight = grid("weight.bin") / 1000.0
fig, axes = plt.subplots(3, 1, figsize=(11, 15.5))
anomaly_map(axes[0], weight, 1.0,
            "Where the oscillation is felt: local °C per 1 °C of index",
            "°C per °C of index")
anomaly_map(axes[1], weight * raw_index[warm], 2.5,
            f"Strongest warm phase (index {raw_index[warm]:+.1f} °C, {date_of(day[warm])}): temperature anomaly")
anomaly_map(axes[2], weight * raw_index[cold], 2.5,
            f"Strongest cold phase (index {raw_index[cold]:+.1f} °C, {date_of(day[cold])}): temperature anomaly")
fig.tight_layout()
fig.savefig(f"{OUT}/2-enso-maps.png", dpi=100, bbox_inches="tight", pad_inches=0.25)
plt.close(fig)

# ---------------------------------------------------------------- 50 years: relief + ice
a = Image.open(f"{D}/relief_year1.png"); b = Image.open(f"{D}/relief_year51.png")
cov1, cov51 = grid("cover_year1.bin"), grid("cover_year51.bin")
ICE = {5, 6}  # SeaIce, Glacier (see Cover enum order)
SNOWISH = {2, 3, 4, 5, 6}  # Snow, RiverIce, LakeIce, SeaIce, Glacier
i1, i51 = np.isin(cov1, list(SNOWISH)), np.isin(cov51, list(SNOWISH))
change = np.full((H, W), 0)
change[land | (water == 2)] = 1
change[i1 & i51] = 2
change[~i1 & i51] = 3
change[i1 & ~i51] = 4
cmap = ListedColormap(["#d5e3f3", "#e8e6dc", "#b9b8b1", BLUE, RED])
fig = plt.figure(figsize=(12, 13))
gs = fig.add_gridspec(3, 1, height_ratios=[1, 1, 1.05])
for k, (img, title) in enumerate(((a, f"Year 1, northern mid-winter (day 319)"),
                                  (b, f"Year 51, same day: 50 years later"))):
    ax = fig.add_subplot(gs[k]); ax.imshow(img); ax.set_title(title); ax.axis("off")
ax = fig.add_subplot(gs[2])
ax.imshow(change, cmap=cmap, vmin=-0.5, vmax=4.5, interpolation="nearest")
coastline(ax); lat_axis(ax)
ax.set_title("Ice and snow cover: year 51 vs year 1, same day")
from matplotlib.patches import Patch
ax.legend(handles=[Patch(color="#b9b8b1", label=f"frozen both years"),
                   Patch(color=BLUE, label=f"newly frozen ({(~i1 & i51).sum()} tiles)"),
                   Patch(color=RED, label=f"thawed ({(i1 & ~i51).sum()} tiles)")],
          loc="lower center", bbox_to_anchor=(0.5, -0.16), ncol=3, frameon=False,
          labelcolor=INK2)
fig.tight_layout()
fig.savefig(f"{OUT}/3-fifty-years-ice.png", dpi=100, bbox_inches="tight", pad_inches=0.25)
plt.close(fig)

# ---------------------------------------------------------------- 50 years: temperature & trends
t1 = grid("annual_mean_year1.bin") / 10; t50 = grid("annual_mean_year50.bin") / 10
annual = list(csv.DictReader(open(f"{D}/annual.csv")))
yr = np.array([int(r["year"]) for r in annual])
fig = plt.figure(figsize=(12, 11))
gs = fig.add_gridspec(3, 2, height_ratios=[1.6, 1, 1])
ax = fig.add_subplot(gs[0, :])
idx1 = raw_index[:DAYS].mean(); idx50 = raw_index[49 * DAYS:50 * DAYS].mean()
anomaly_map(ax, t50 - t1, 2.5,
            f"Annual mean temperature, year 50 minus year 1 "
            f"(mean index {idx1:+.1f} → {idx50:+.1f} °C, drift {drift[DAYS//2]:+.1f} → {drift[49*DAYS+DAYS//2]:+.1f} °C)")
panels = [("glacier_land_pct", "Glaciers, % of land area", "%"),
          ("permafrost_land_pct", "Permafrost, % of land area", "%"),
          ("fertility_land_mean", "Mean land fertility", "% of max"),
          ("population", "Total population (24 cities)", "people")]
for k, (key, title, unit) in enumerate(panels):
    ax = fig.add_subplot(gs[1 + k // 2, k % 2])
    v = np.array([float(r[key]) for r in annual])
    ax.plot(yr, v, color=BLUE, lw=2)
    ax.plot(yr[[0, -1]], v[[0, -1]], "o", ms=7, color=BLUE, mec=SURFACE, mew=2)
    for i, ha in ((0, "left"), (-1, "right")):
        ax.annotate(f"{v[i]:,.1f}" if key != "population" else f"{int(v[i]):,}",
                    (yr[i], v[i]), textcoords="offset points", xytext=(0, 9), ha=ha,
                    color=INK2, fontsize=9)
    ax.set_title(title); ax.set_ylabel(unit)
    lo, hi = v.min(), v.max(); pad = (hi - lo) * 0.25 + (0.5 if key != "population" else 50)
    ax.set_ylim(lo - pad, hi + pad)
    if k >= 2:
        ax.set_xlabel("simulated year")
fig.tight_layout()
fig.savefig(f"{OUT}/4-fifty-years-trends.png", dpi=100, bbox_inches="tight", pad_inches=0.25)
plt.close(fig)
print("ok")
