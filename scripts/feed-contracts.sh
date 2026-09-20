#!/usr/bin/env bash
set -euo pipefail

check_prefixes() {
  local url=$1
  shift
  local listing
  listing=$(curl --fail --silent --show-error --retry 3 "$url")
  for prefix in "$@"; do
    grep -Fq "<Prefix>${prefix}</Prefix>" <<<"$listing" || {
      echo "missing provider prefix: $prefix" >&2
      return 1
    }
  done
}

check_prefixes \
  'https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&delimiter=/&prefix=CONUS/' \
  'CONUS/MergedReflectivityQCComposite_00.50/' \
  'CONUS/LowLevelCompositeReflectivity_00.50/' \
  'CONUS/MergedAzShear_0-2kmAGL_00.50/' \
  'CONUS/MergedAzShear_3-6kmAGL_00.50/' \
  'CONUS/POSH_00.50/' \
  'CONUS/MultiSensor_QPE_01H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_03H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_06H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_12H_Pass2_00.00/' \
  'CONUS/NLDN_CG_005min_AvgDensity_00.00/'

check_prefixes \
  'https://noaa-goes19.s3.amazonaws.com/?list-type=2&delimiter=/' \
  'ABI-L2-CMIPC/' \
  'ABI-L2-CMIPM/' \
  'GLM-L2-LCFA/'
