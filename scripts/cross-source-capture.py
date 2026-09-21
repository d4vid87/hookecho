#!/usr/bin/env python3
"""Capture latest official radar, MRMS, ABI, and GLM valid times as one JSON artifact."""

import json
import re
import sys
import time
import urllib.parse
import urllib.request
import xml.etree.ElementTree as ET
from datetime import datetime, timedelta, timezone
from pathlib import Path

UTC = timezone.utc
S3 = {
    "radar": ("https://unidata-nexrad-level2.s3.amazonaws.com", "{ymd}/KTLX/"),
    "mrms": ("https://noaa-mrms-pds.s3.amazonaws.com", "CONUS/MergedReflectivityQCComposite_00.50/{ymd}/"),
    "abi": ("https://noaa-goes19.s3.amazonaws.com", "ABI-L2-CMIPC/{year}/{doy}/{hour}/"),
    "glm": ("https://noaa-goes19.s3.amazonaws.com", "GLM-L2-LCFA/{year}/{doy}/{hour}/"),
}
MAX_AGE = {"radar": 20, "mrms": 15, "abi": 15, "glm": 10}


def fetch_listing(base: str, prefix: str) -> list[dict[str, str]]:
    query = urllib.parse.urlencode({"list-type": "2", "max-keys": "1000", "prefix": prefix})
    error = None
    for attempt in range(3):
        try:
            with urllib.request.urlopen(f"{base}/?{query}", timeout=30) as response:
                root = ET.fromstring(response.read())
            ns = {"s3": "http://s3.amazonaws.com/doc/2006-03-01/"}
            return [
                {"key": item.findtext("s3:Key", "", ns), "received": item.findtext("s3:LastModified", "", ns)}
                for item in root.findall("s3:Contents", ns)
            ]
        except Exception as exc:  # scheduled network probe: retry transport/provider failures
            error = exc
            time.sleep(attempt + 1)
    raise RuntimeError(f"listing failed for {prefix}: {error}")


def valid_time(kind: str, key: str) -> datetime | None:
    if kind == "radar":
        match = re.search(r"K[A-Z0-9]{3}(\d{8})_(\d{6})_V\d\d$", key)
        fmt = "%Y%m%d%H%M%S"
    elif kind == "mrms":
        match = re.search(r"_(\d{8})-(\d{6})\.grib2", key)
        fmt = "%Y%m%d%H%M%S"
    else:
        match = re.search(r"_s(\d{4})(\d{3})(\d{6})", key)
        fmt = "%Y%j%H%M%S"
    if not match:
        return None
    return datetime.strptime("".join(match.groups()), fmt).replace(tzinfo=UTC)


def prefix(kind: str, when: datetime) -> str:
    return S3[kind][1].format(
        ymd=when.strftime("%Y/%m/%d") if kind == "radar" else when.strftime("%Y%m%d"),
        year=when.strftime("%Y"),
        doy=when.strftime("%j"),
        hour=when.strftime("%H"),
    )


def latest(kind: str, now: datetime) -> dict[str, object]:
    base, _ = S3[kind]
    candidates = []
    for back in (0, 1):
        when = now - (timedelta(days=back) if kind in {"radar", "mrms"} else timedelta(hours=back))
        for item in fetch_listing(base, prefix(kind, when)):
            valid = valid_time(kind, item["key"])
            if valid is not None:
                candidates.append((valid, item))
        if candidates:
            break
    if not candidates:
        raise RuntimeError(f"no parseable recent {kind} objects")
    valid, item = max(candidates, key=lambda pair: pair[0])
    return {
        "key": item["key"],
        "valid_time": valid.isoformat().replace("+00:00", "Z"),
        "received_time": item["received"],
        "age_minutes": round((now - valid).total_seconds() / 60, 2),
    }


def self_test() -> None:
    cases = {
        "radar": ("2026/09/20/KTLX/KTLX20260920_235501_V06", "2026-09-20T23:55:01Z"),
        "mrms": ("CONUS/X/20260920/MRMS_X_20260920-235800.grib2.gz", "2026-09-20T23:58:00Z"),
        "abi": ("ABI-L2-CMIPC/2026/263/23/OR_ABI-L2-CMIPC-M6C13_G19_s2026263235900.nc", "2026-09-20T23:59:00Z"),
        "glm": ("GLM-L2-LCFA/2026/263/23/OR_GLM-L2-LCFA_G19_s2026263235950.nc", "2026-09-20T23:59:50Z"),
    }
    for kind, (key, expected) in cases.items():
        assert valid_time(kind, key).isoformat().replace("+00:00", "Z") == expected


def main() -> int:
    if "--self-test" in sys.argv:
        self_test()
        return 0
    now = datetime.now(UTC)
    sources = {kind: latest(kind, now) for kind in S3}
    radar_time = datetime.fromisoformat(str(sources["radar"]["valid_time"]).replace("Z", "+00:00"))
    for source in sources.values():
        valid = datetime.fromisoformat(str(source["valid_time"]).replace("Z", "+00:00"))
        source["offset_from_radar_minutes"] = round((valid - radar_time).total_seconds() / 60, 2)
    capture = {
        "captured_at": now.isoformat().replace("+00:00", "Z"),
        "sources": sources,
        "policies": {"observed_imagery": "NearestPast", "maximum_age_minutes": MAX_AGE},
    }
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "cross-source-capture.json")
    out.write_text(json.dumps(capture, indent=2) + "\n")
    stale = [kind for kind, source in sources.items() if source["age_minutes"] > MAX_AGE[kind]]
    if stale:
        print(f"stale sources: {', '.join(stale)}", file=sys.stderr)
        return 1
    print(out)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
