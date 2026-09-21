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

check_recent_rrfs() {
  local today yesterday body
  today=$(date -u +%Y%m%d)
  yesterday=$(date -u -d '1 day ago' +%Y%m%d)
  body=$(curl --fail --silent --show-error --retry 3 \
    "https://noaa-rrfs-ops-pds.s3.amazonaws.com/?list-type=2&max-keys=2&prefix=rrfs.${today}/")
  if ! grep -Fq '.grib2.idx</Key>' <<<"$body"; then
    body=$(curl --fail --silent --show-error --retry 3 \
      "https://noaa-rrfs-ops-pds.s3.amazonaws.com/?list-type=2&max-keys=2&prefix=rrfs.${yesterday}/")
    grep -Fq '.grib2.idx</Key>' <<<"$body" || {
      echo "no recent RRFS index object" >&2
      return 1
    }
  fi
}

check_recent_gefs() {
  local day cycle product body found complete
  found=0
  for day in "$(date -u +%Y%m%d)" "$(date -u -d '1 day ago' +%Y%m%d)"; do
    for cycle in 18 12 06 00; do
      complete=1
      for product in geavg gespr gec00 gep01 gep30; do
        body=$(curl --fail --silent --show-error --retry 3 \
          "https://noaa-gefs-pds.s3.amazonaws.com/?list-type=2&max-keys=1&prefix=gefs.${day}/${cycle}/atmos/pgrb2sp25/${product}.t${cycle}z.pgrb2s.0p25.f000.idx")
        if ! grep -Fq '<Key>' <<<"$body"; then
          complete=0
          break
        fi
      done
      if (( complete == 1 )); then
        found=1
        break 2
      fi
    done
  done
  (( found == 1 )) || {
    echo "no recent GEFS aggregate/member index objects" >&2
    return 1
  }
}

check_recent_rtma() {
  local age day hour url body
  for age in 1 2 3 4 5 6; do
    day=$(date -u -d "$age hour ago" +%Y%m%d)
    hour=$(date -u -d "$age hour ago" +%H)
    url="https://nomads.ncep.noaa.gov/pub/data/nccf/com/rtma/v2.10/rtma2p5.${day}/rtma2p5.t${hour}z.2dvaranl_ndfd.grb2_wexp.idx"
    if body=$(curl --fail --silent --show-error --retry 2 "$url"); then
      for field in 'PRES:surface' 'TMP:2 m above ground' 'DPT:2 m above ground' 'UGRD:10 m above ground'; do
        grep -Fq "$field" <<<"$body" || {
          echo "RTMA index missing field: $field" >&2
          return 1
        }
      done
      return 0
    fi
  done
  echo "no recent RTMA index" >&2
  return 1
}

check_recent_urma() {
  local age day hour url header
  for age in $(seq 6 48); do
    day=$(date -u -d "$age hour ago" +%Y%m%d)
    hour=$(date -u -d "$age hour ago" +%H)
    url="https://nomads.ncep.noaa.gov/pub/data/nccf/com/urma/prod/urma2p5.${day}/urma2p5.t${hour}z.2dvaranl_ndfd.grb2_wexp"
    if header=$(curl --fail --silent --show-error --retry 2 --range 0-3 "$url"); then
      [[ $header == GRIB ]] || {
        echo "recent URMA object is not GRIB2" >&2
        return 1
      }
      return 0
    fi
  done
  echo "no recent range-readable URMA analysis" >&2
  return 1
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
  'CONUS/RadarOnly_QPE_48H_00.00/' \
  'CONUS/RadarOnly_QPE_72H_00.00/' \
  'CONUS/LayerCompositeReflectivity_Low_00.50/' \
  'CONUS/LayerCompositeReflectivity_High_00.50/' \
  'CONUS/LayerCompositeReflectivity_Super_00.50/' \
  'CONUS/Reflectivity_0C_00.50/' \
  'CONUS/Reflectivity_-10C_00.50/' \
  'CONUS/Reflectivity_-20C_00.50/' \
  'CONUS/HeightCompositeReflectivity_00.50/' \
  'CONUS/HeightLowLevelCompositeReflectivity_00.50/' \
  'CONUS/SeamlessHSRHeight_00.00/' \
  'CONUS/BREF_1HR_MAX_00.50/' \
  'CONUS/CREF_1HR_MAX_00.50/' \
  'CONUS/BrightBandBottomHeight_00.00/' \
  'CONUS/BrightBandTopHeight_00.00/' \
  'CONUS/Model_0degC_Height_00.50/' \
  'CONUS/RadarOnly_QPE_15M_00.00/' \
  'CONUS/Reflectivity_-5C_00.50/' \
  'CONUS/Reflectivity_-15C_00.50/' \
  'CONUS/SeamlessHSR_00.00/' \
  'CONUS/MergedReflectivityQC_00.50/' \
  'CONUS/MergedReflectivityQC_00.75/' \
  'CONUS/MergedReflectivityQC_01.00/' \
  'CONUS/MergedReflectivityQC_01.25/' \
  'CONUS/MergedReflectivityQC_01.50/' \
  'CONUS/MergedReflectivityQC_01.75/' \
  'CONUS/MergedReflectivityQC_02.00/' \
  'CONUS/MergedReflectivityQC_02.25/' \
  'CONUS/MergedReflectivityQC_02.50/' \
  'CONUS/MergedReflectivityQC_02.75/' \
  'CONUS/MergedReflectivityQC_03.00/' \
  'CONUS/MergedReflectivityQC_03.50/' \
  'CONUS/MergedReflectivityQC_04.00/' \
  'CONUS/MergedReflectivityQC_04.50/' \
  'CONUS/MergedReflectivityQC_05.00/' \
  'CONUS/MergedReflectivityQC_05.50/' \
  'CONUS/MergedReflectivityQC_06.00/' \
  'CONUS/MergedReflectivityQC_06.50/' \
  'CONUS/MergedReflectivityQC_07.00/' \
  'CONUS/MergedReflectivityQC_07.50/' \
  'CONUS/RotationTrack30min_00.50/' \
  'CONUS/RotationTrack60min_00.50/' \
  'CONUS/RotationTrack120min_00.50/' \
  'CONUS/RotationTrack240min_00.50/' \
  'CONUS/RotationTrack360min_00.50/' \
  'CONUS/RotationTrack1440min_00.50/' \
  'CONUS/FLASH_QPE_ARI01H_00.00/' \
  'CONUS/FLASH_QPE_ARI03H_00.00/' \
  'CONUS/FLASH_QPE_ARI06H_00.00/' \
  'CONUS/FLASH_QPE_ARI12H_00.00/' \
  'CONUS/FLASH_QPE_ARI24H_00.00/' \
  'CONUS/FLASH_QPE_ARIMAX_00.00/' \
  'CONUS/FLASH_QPE_FFG01H_00.00/' \
  'CONUS/FLASH_QPE_FFG03H_00.00/' \
  'CONUS/FLASH_QPE_FFG06H_00.00/' \
  'CONUS/FLASH_QPE_FFGMAX_00.00/' \
  'CONUS/FLASH_CREST_MAXUNITSTREAMFLOW_00.00/' \
  'CONUS/FLASH_CREST_MAXSOILSAT_00.00/' \
  'CONUS/MultiSensor_QPE_01H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_03H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_06H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_12H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_48H_Pass2_00.00/' \
  'CONUS/MultiSensor_QPE_72H_Pass2_00.00/' \
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
  'CONUS/RadarOnly_QPE_48H_00.00' \
  'CONUS/RadarOnly_QPE_72H_00.00' \
  'CONUS/LayerCompositeReflectivity_Low_00.50' \
  'CONUS/LayerCompositeReflectivity_High_00.50' \
  'CONUS/LayerCompositeReflectivity_Super_00.50' \
  'CONUS/Reflectivity_0C_00.50' \
  'CONUS/Reflectivity_-10C_00.50' \
  'CONUS/Reflectivity_-20C_00.50' \
  'CONUS/HeightCompositeReflectivity_00.50' \
  'CONUS/HeightLowLevelCompositeReflectivity_00.50' \
  'CONUS/SeamlessHSRHeight_00.00' \
  'CONUS/BREF_1HR_MAX_00.50' \
  'CONUS/CREF_1HR_MAX_00.50' \
  'CONUS/BrightBandBottomHeight_00.00' \
  'CONUS/BrightBandTopHeight_00.00' \
  'CONUS/Model_0degC_Height_00.50' \
  'CONUS/RadarOnly_QPE_15M_00.00' \
  'CONUS/Reflectivity_-5C_00.50' \
  'CONUS/Reflectivity_-15C_00.50' \
  'CONUS/SeamlessHSR_00.00' \
  'CONUS/MergedReflectivityQC_00.50' \
  'CONUS/MergedReflectivityQC_00.75' \
  'CONUS/MergedReflectivityQC_01.00' \
  'CONUS/MergedReflectivityQC_01.25' \
  'CONUS/MergedReflectivityQC_01.50' \
  'CONUS/MergedReflectivityQC_01.75' \
  'CONUS/MergedReflectivityQC_02.00' \
  'CONUS/MergedReflectivityQC_02.25' \
  'CONUS/MergedReflectivityQC_02.50' \
  'CONUS/MergedReflectivityQC_02.75' \
  'CONUS/MergedReflectivityQC_03.00' \
  'CONUS/MergedReflectivityQC_03.50' \
  'CONUS/MergedReflectivityQC_04.00' \
  'CONUS/MergedReflectivityQC_04.50' \
  'CONUS/MergedReflectivityQC_05.00' \
  'CONUS/MergedReflectivityQC_05.50' \
  'CONUS/MergedReflectivityQC_06.00' \
  'CONUS/MergedReflectivityQC_06.50' \
  'CONUS/MergedReflectivityQC_07.00' \
  'CONUS/MergedReflectivityQC_07.50' \
  'CONUS/RotationTrack30min_00.50' \
  'CONUS/RotationTrack60min_00.50' \
  'CONUS/RotationTrack120min_00.50' \
  'CONUS/RotationTrack240min_00.50' \
  'CONUS/RotationTrack360min_00.50' \
  'CONUS/RotationTrack1440min_00.50' \
  'CONUS/FLASH_QPE_ARI01H_00.00' \
  'CONUS/FLASH_QPE_ARI03H_00.00' \
  'CONUS/FLASH_QPE_ARI06H_00.00' \
  'CONUS/FLASH_QPE_ARI12H_00.00' \
  'CONUS/FLASH_QPE_ARI24H_00.00' \
  'CONUS/FLASH_QPE_ARIMAX_00.00' \
  'CONUS/FLASH_QPE_FFG01H_00.00' \
  'CONUS/FLASH_QPE_FFG03H_00.00' \
  'CONUS/FLASH_QPE_FFG06H_00.00' \
  'CONUS/FLASH_QPE_FFGMAX_00.00' \
  'CONUS/FLASH_CREST_MAXUNITSTREAMFLOW_00.00' \
  'CONUS/FLASH_CREST_MAXSOILSAT_00.00' \
  'CONUS/MultiSensor_QPE_01H_Pass2_00.00' \
  'CONUS/MultiSensor_QPE_03H_Pass2_00.00' \
  'CONUS/MultiSensor_QPE_06H_Pass2_00.00' \
  'CONUS/MultiSensor_QPE_12H_Pass2_00.00' \
  'CONUS/MultiSensor_QPE_48H_Pass2_00.00' \
  'CONUS/MultiSensor_QPE_72H_Pass2_00.00' \
  'CONUS/NLDN_CG_005min_AvgDensity_00.00'

check_prefixes \
  'https://noaa-goes19.s3.amazonaws.com/?list-type=2&delimiter=/' \
  'ABI-L2-CMIPC/' \
  'ABI-L2-CMIPF/' \
  'ABI-L2-CMIPM/' \
  'GLM-L2-LCFA/'

check_recent_goes 'ABI-L2-CMIPC'
check_recent_goes 'ABI-L2-CMIPF'
check_recent_goes 'ABI-L2-CMIPM'
check_recent_goes 'GLM-L2-LCFA'
check_recent_rrfs
check_recent_gefs
check_recent_rtma
check_recent_urma
