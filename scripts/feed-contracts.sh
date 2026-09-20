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

check_recent_mrms() {
  local today yesterday product body
  today=$(date -u +%Y%m%d)
  yesterday=$(date -u -d '1 day ago' +%Y%m%d)
  for product in "$@"; do
    body=$(curl --fail --silent --show-error --retry 3 \
      "https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&max-keys=1&prefix=${product}/${today}/")
    if ! grep -Fq '<Key>' <<<"$body"; then
      body=$(curl --fail --silent --show-error --retry 3 \
        "https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&max-keys=1&prefix=${product}/${yesterday}/")
      grep -Fq '<Key>' <<<"$body" || {
        echo "no recent MRMS object: $product" >&2
        return 1
      }
    fi
  done
}

check_recent_goes() {
  local product=$1 hour previous body
  hour=$(date -u +%Y/%j/%H)
  previous=$(date -u -d '1 hour ago' +%Y/%j/%H)
  body=$(curl --fail --silent --show-error --retry 3 \
    "https://noaa-goes19.s3.amazonaws.com/?list-type=2&max-keys=1&prefix=${product}/${hour}/")
  if ! grep -Fq '<Key>' <<<"$body"; then
    body=$(curl --fail --silent --show-error --retry 3 \
      "https://noaa-goes19.s3.amazonaws.com/?list-type=2&max-keys=1&prefix=${product}/${previous}/")
    grep -Fq '<Key>' <<<"$body" || {
      echo "no recent GOES-19 object: $product" >&2
      return 1
    }
  fi
}

check_prefixes \
  'https://noaa-mrms-pds.s3.amazonaws.com/?list-type=2&delimiter=/&prefix=CONUS/' \
  'CONUS/MergedReflectivityQCComposite_00.50/' \
  'CONUS/LowLevelCompositeReflectivity_00.50/' \
  'CONUS/MergedAzShear_0-2kmAGL_00.50/' \
  'CONUS/MergedAzShear_3-6kmAGL_00.50/' \
  'CONUS/POSH_00.50/' \
  'CONUS/EchoTop_18_00.50/' \
  'CONUS/EchoTop_30_00.50/' \
  'CONUS/EchoTop_50_00.50/' \
  'CONUS/EchoTop_60_00.50/' \
  'CONUS/LVL3_HighResVIL_00.50/' \
  'CONUS/VIL_Density_00.50/' \
  'CONUS/RadarOnly_QPE_01H_00.00/' \
  'CONUS/RadarOnly_QPE_03H_00.00/' \
  'CONUS/RadarOnly_QPE_06H_00.00/' \
  'CONUS/RadarOnly_QPE_12H_00.00/' \
  'CONUS/RadarOnly_QPE_24H_00.00/' \
  'CONUS/LayerCompositeReflectivity_Low_00.50/' \
  'CONUS/LayerCompositeReflectivity_High_00.50/' \
  'CONUS/LayerCompositeReflectivity_Super_00.50/' \
  'CONUS/FLASH_QPE_ARI01H_00.00/' \
  'CONUS/FLASH_QPE_ARI03H_00.00/' \
  'CONUS/FLASH_QPE_ARI06H_00.00/' \
  'CONUS/FLASH_QPE_ARI12H_00.00/' \
  'CONUS/FLASH_QPE_ARI24H_00.00/' \
  'CONUS/FLASH_QPE_ARIMAX_00.00/' \
  'CONUS/MultiSensor_QPE_01H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_03H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_06H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_12H_Pass2_00.00/' \
  'CONUS/NLDN_CG_005min_AvgDensity_00.00/'

check_recent_mrms \
  'CONUS/MergedReflectivityQCComposite_00.50' \
  'CONUS/LowLevelCompositeReflectivity_00.50' \
  'CONUS/MergedAzShear_0-2kmAGL_00.50' \
  'CONUS/MergedAzShear_3-6kmAGL_00.50' \
  'CONUS/POSH_00.50' \
  'CONUS/EchoTop_18_00.50' \
  'CONUS/EchoTop_30_00.50' \
  'CONUS/EchoTop_50_00.50' \
  'CONUS/EchoTop_60_00.50' \
  'CONUS/LVL3_HighResVIL_00.50' \
  'CONUS/VIL_Density_00.50' \
  'CONUS/RadarOnly_QPE_01H_00.00' \
  'CONUS/RadarOnly_QPE_03H_00.00' \
  'CONUS/RadarOnly_QPE_06H_00.00' \
  'CONUS/RadarOnly_QPE_12H_00.00' \
  'CONUS/RadarOnly_QPE_24H_00.00' \
  'CONUS/LayerCompositeReflectivity_Low_00.50' \
  'CONUS/LayerCompositeReflectivity_High_00.50' \
  'CONUS/LayerCompositeReflectivity_Super_00.50' \
  'CONUS/FLASH_QPE_ARI01H_00.00' \
  'CONUS/FLASH_QPE_ARI03H_00.00' \
  'CONUS/FLASH_QPE_ARI06H_00.00' \
  'CONUS/FLASH_QPE_ARI12H_00.00' \
  'CONUS/FLASH_QPE_ARI24H_00.00' \
  'CONUS/FLASH_QPE_ARIMAX_00.00' \
  'CONUS/MultiSensor_QPE_01H_Pass2_00.00' \
  'CONUS/MultiSensor_QPE_03H_Pass2_00.00' \
  'CONUS/MultiSensor_QPE_06H_Pass2_00.00' \
  'CONUS/MultiSensor_QPE_12H_Pass2_00.00' \
  'CONUS/NLDN_CG_005min_AvgDensity_00.00'

check_prefixes \
  'https://noaa-goes19.s3.amazonaws.com/?list-type=2&delimiter=/' \
  'ABI-L2-CMIPC/' \
  'ABI-L2-CMIPM/' \
  'GLM-L2-LCFA/'

check_recent_goes 'ABI-L2-CMIPC'
check_recent_goes 'ABI-L2-CMIPM'
check_recent_goes 'GLM-L2-LCFA'
