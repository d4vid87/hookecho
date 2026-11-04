use crate::data::{CFPMomentValue, DataMoment, GateStatus, MomentValue, Product, Radial};
use crate::geo::{GeoExtent, GeoPoint};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A 2D polar field of floating-point values for one sweep of one product.
///
/// This is the universal data surface consumed by both the renderer and processing
/// algorithms. It can be constructed from raw decoded radials or produced as the
/// output of a processing step.
///
/// Values are stored as parallel `f32` and [`GateStatus`] arrays in row-major order:
/// `values[azimuth_index * gate_count + gate_index]`. This layout provides contiguous
/// float slices for efficient numerical processing.
///
/// # Construction
///
/// ```ignore
/// use nexrad_model::data::{SweepField, Product};
///
/// let field = SweepField::from_radials(sweep.radials(), Product::Reflectivity)
///     .expect("reflectivity data present");
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SweepField {
    /// Human-readable label (e.g., "Reflectivity", "Dealiased Velocity").
    label: String,
    /// Physical unit string (e.g., "dBZ", "m/s").
    unit: String,
    /// Elevation angle in degrees.
    elevation_degrees: f32,
    /// Azimuth angle in degrees for each radial row, sorted ascending.
    azimuths: Vec<f32>,
    /// Azimuthal spacing between radials in degrees.
    azimuth_spacing_degrees: f32,
    /// Range to center of first gate in km.
    first_gate_range_km: f64,
    /// Gate spacing in km.
    gate_interval_km: f64,
    /// Number of gates per radial.
    gate_count: usize,
    /// Flat array of gate values, row-major: `values[az_idx * gate_count + gate_idx]`.
    values: Vec<f32>,
    /// Parallel status array: `status[az_idx * gate_count + gate_idx]`.
    status: Vec<GateStatus>,
}

impl SweepField {
    /// Create a `SweepField` from decoded radials for a specific product.
    ///
    /// Extracts and decodes moment data from each radial, converting [`MomentValue`]
    /// into `(f32, GateStatus)` pairs. Radials are sorted by azimuth angle.
    ///
    /// Returns `None` if no radials contain data for the requested product.
    pub fn from_radials(radials: &[Radial], product: Product) -> Option<Self> {
        if radials.is_empty() {
            return None;
        }

        let (first_gate_range_km, gate_interval_km, gate_count) =
            Self::extract_gate_params(radials, product)?;

        let elevation_degrees = radials[0].elevation_angle_degrees();
        let azimuth_spacing_degrees = radials[0].azimuth_spacing_degrees();

        // Collect (azimuth, radial_index) for radials that have data, then sort by
        // azimuth. This avoids allocating temporary per-radial Vec<f32>/Vec<GateStatus>
        // buffers — we decode directly into the final flat arrays.
        let mut indexed: Vec<(f32, usize)> = radials
            .iter()
            .enumerate()
            .filter(|(_, r)| Self::radial_has_product(r, product))
            .map(|(i, r)| (r.azimuth_angle_degrees(), i))
            .collect();

        if indexed.is_empty() {
            return None;
        }

        indexed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let azimuth_count = indexed.len();
        let total_gates = azimuth_count * gate_count;
        let mut azimuths = Vec::with_capacity(azimuth_count);
        let mut values = vec![0.0f32; total_gates];
        let mut status = vec![GateStatus::NoData; total_gates];

        for (row, &(az, radial_idx)) in indexed.iter().enumerate() {
            azimuths.push(az);
            let row_offset = row * gate_count;
            Self::decode_moment_into(
                &radials[radial_idx],
                product,
                &mut values[row_offset..row_offset + gate_count],
                &mut status[row_offset..row_offset + gate_count],
            );
        }

        Some(Self {
            label: product.label().to_string(),
            unit: product.unit().to_string(),
            elevation_degrees,
            azimuths,
            azimuth_spacing_degrees,
            first_gate_range_km,
            gate_interval_km,
            gate_count,
            values,
            status,
        })
    }

    /// Create a `SweepField` from decoded radials, consuming the radials.
    ///
    /// Same as [`from_radials`](Self::from_radials) but takes ownership of the radials,
    /// allowing them to be dropped immediately after extraction. Each radial's encoded
    /// moment data is freed as soon as it has been decoded into the field, reducing
    /// peak memory usage when you no longer need the original radials.
    pub fn from_radials_owned(mut radials: Vec<Radial>, product: Product) -> Option<Self> {
        if radials.is_empty() {
            return None;
        }

        let (first_gate_range_km, gate_interval_km, gate_count) =
            Self::extract_gate_params(&radials, product)?;

        let elevation_degrees = radials[0].elevation_angle_degrees();
        let azimuth_spacing_degrees = radials[0].azimuth_spacing_degrees();

        // Build sort order by azimuth. We store the original index so we can
        // take radials out of the Vec and drop them after decoding.
        let mut indexed: Vec<(f32, usize)> = radials
            .iter()
            .enumerate()
            .filter(|(_, r)| Self::radial_has_product(r, product))
            .map(|(i, r)| (r.azimuth_angle_degrees(), i))
            .collect();

        if indexed.is_empty() {
            return None;
        }

        indexed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

        let azimuth_count = indexed.len();
        let total_gates = azimuth_count * gate_count;
        let mut azimuths = Vec::with_capacity(azimuth_count);
        let mut values = vec![0.0f32; total_gates];
        let mut status = vec![GateStatus::NoData; total_gates];

        // Process in reverse index order so that swap_remove doesn't invalidate
        // indices we haven't visited yet.
        // Build (row_in_output, original_index) pairs sorted by original_index descending.
        let mut work: Vec<(usize, usize)> = indexed
            .iter()
            .enumerate()
            .map(|(row, &(_, orig_idx))| (row, orig_idx))
            .collect();
        work.sort_by(|a, b| b.1.cmp(&a.1));

        // Stash azimuths in order (we already have them from indexed).
        for &(az, _) in &indexed {
            azimuths.push(az);
        }

        for &(row, orig_idx) in &work {
            let radial = radials.swap_remove(orig_idx);
            let row_offset = row * gate_count;
            Self::decode_moment_into(
                &radial,
                product,
                &mut values[row_offset..row_offset + gate_count],
                &mut status[row_offset..row_offset + gate_count],
            );
            // `radial` is dropped here, freeing its encoded moment bytes
        }

        Some(Self {
            label: product.label().to_string(),
            unit: product.unit().to_string(),
            elevation_degrees,
            azimuths,
            azimuth_spacing_degrees,
            first_gate_range_km,
            gate_interval_km,
            gate_count,
            values,
            status,
        })
    }

    /// Create an empty field with the given geometry (for processing output).
    ///
    /// All values are initialized to `0.0` with [`GateStatus::NoData`].
    pub fn new_empty(
        label: impl Into<String>,
        unit: impl Into<String>,
        elevation_degrees: f32,
        azimuths: Vec<f32>,
        azimuth_spacing_degrees: f32,
        first_gate_range_km: f64,
        gate_interval_km: f64,
        gate_count: usize,
    ) -> Self {
        let total = azimuths.len() * gate_count;
        Self {
            label: label.into(),
            unit: unit.into(),
            elevation_degrees,
            azimuths,
            azimuth_spacing_degrees,
            first_gate_range_km,
            gate_interval_km,
            gate_count,
            values: vec![0.0; total],
            status: vec![GateStatus::NoData; total],
        }
    }

    /// Create a new field with the same geometry as this one but empty data.
    pub fn new_like(&self, label: impl Into<String>, unit: impl Into<String>) -> Self {
        Self::new_empty(
            label,
            unit,
            self.elevation_degrees,
            self.azimuths.clone(),
            self.azimuth_spacing_degrees,
            self.first_gate_range_km,
            self.gate_interval_km,
            self.gate_count,
        )
    }

    // --- Accessors ---

    /// Human-readable label for this field.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Physical unit string.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// Elevation angle in degrees.
    pub fn elevation_degrees(&self) -> f32 {
        self.elevation_degrees
    }

    /// Number of azimuth rows.
    pub fn azimuth_count(&self) -> usize {
        self.azimuths.len()
    }

    /// Number of gates per radial.
    pub fn gate_count(&self) -> usize {
        self.gate_count
    }

    /// Azimuth angles for each radial row (sorted ascending).
    pub fn azimuths(&self) -> &[f32] {
        &self.azimuths
    }

    /// Azimuthal spacing between radials in degrees.
    pub fn azimuth_spacing_degrees(&self) -> f32 {
        self.azimuth_spacing_degrees
    }

    /// Range to center of first gate in km.
    pub fn first_gate_range_km(&self) -> f64 {
        self.first_gate_range_km
    }

    /// Gate spacing in km.
    pub fn gate_interval_km(&self) -> f64 {
        self.gate_interval_km
    }

    /// Maximum range of the data in km.
    pub fn max_range_km(&self) -> f64 {
        self.first_gate_range_km + (self.gate_count as f64) * self.gate_interval_km
    }

    /// All gate values as a flat slice (row-major: `values[az_idx * gate_count + gate_idx]`).
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// All gate statuses as a flat slice.
    pub fn statuses(&self) -> &[GateStatus] {
        &self.status
    }

    // --- Per-gate access ---

    /// Get the value and status at a specific (azimuth_index, gate_index).
    pub fn get(&self, azimuth_idx: usize, gate_idx: usize) -> (f32, GateStatus) {
        let idx = azimuth_idx * self.gate_count + gate_idx;
        (self.values[idx], self.status[idx])
    }

    /// Set the value and status at a specific (azimuth_index, gate_index).
    pub fn set(&mut self, azimuth_idx: usize, gate_idx: usize, value: f32, status: GateStatus) {
        let idx = azimuth_idx * self.gate_count + gate_idx;
        self.values[idx] = value;
        self.status[idx] = status;
    }

    /// Get a mutable slice of values for a full radial row.
    pub fn radial_values_mut(&mut self, azimuth_idx: usize) -> &mut [f32] {
        let start = azimuth_idx * self.gate_count;
        &mut self.values[start..start + self.gate_count]
    }

    /// Get a mutable slice of statuses for a full radial row.
    pub fn radial_status_mut(&mut self, azimuth_idx: usize) -> &mut [GateStatus] {
        let start = azimuth_idx * self.gate_count;
        &mut self.status[start..start + self.gate_count]
    }

    /// Get an immutable slice of values for a full radial row.
    pub fn radial_values(&self, azimuth_idx: usize) -> &[f32] {
        let start = azimuth_idx * self.gate_count;
        &self.values[start..start + self.gate_count]
    }

    /// Get an immutable slice of statuses for a full radial row.
    pub fn radial_statuses(&self, azimuth_idx: usize) -> &[GateStatus] {
        let start = azimuth_idx * self.gate_count;
        &self.status[start..start + self.gate_count]
    }

    // --- Queries ---

    /// Query the value at a polar coordinate (finds nearest azimuth and gate).
    ///
    /// Returns `None` if the coordinate is outside the field's range.
    pub fn value_at_polar(&self, azimuth_degrees: f32, range_km: f64) -> Option<(f32, GateStatus)> {
        if range_km < self.first_gate_range_km || range_km >= self.max_range_km() {
            return None;
        }

        let az_idx = self.find_nearest_azimuth(azimuth_degrees)?;
        let gate_idx = ((range_km - self.first_gate_range_km) / self.gate_interval_km) as usize;

        if gate_idx >= self.gate_count {
            return None;
        }

        Some(self.get(az_idx, gate_idx))
    }

    /// Compute the (min, max) of valid values in this field.
    ///
    /// Returns `None` if the field contains no valid values.
    pub fn value_range(&self) -> Option<(f32, f32)> {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        let mut found = false;

        for (i, &val) in self.values.iter().enumerate() {
            if self.status[i] == GateStatus::Valid {
                min = min.min(val);
                max = max.max(val);
                found = true;
            }
        }

        if found {
            Some((min, max))
        } else {
            None
        }
    }

    // --- Internal helpers ---

    /// Find the nearest azimuth index for the given angle.
    fn find_nearest_azimuth(&self, azimuth_degrees: f32) -> Option<usize> {
        if self.azimuths.is_empty() {
            return None;
        }

        let len = self.azimuths.len();
        let pos = self
            .azimuths
            .binary_search_by(|a| {
                a.partial_cmp(&azimuth_degrees)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_else(|i| i);

        if pos == 0 {
            let dist_first = (self.azimuths[0] - azimuth_degrees).abs();
            let dist_last = 360.0 - self.azimuths[len - 1] + azimuth_degrees;
            if dist_last < dist_first {
                Some(len - 1)
            } else {
                Some(0)
            }
        } else if pos >= len {
            let dist_last = (azimuth_degrees - self.azimuths[len - 1]).abs();
            let dist_first = 360.0 - azimuth_degrees + self.azimuths[0];
            if dist_first < dist_last {
                Some(0)
            } else {
                Some(len - 1)
            }
        } else {
            let dist_prev = (azimuth_degrees - self.azimuths[pos - 1]).abs();
            let dist_curr = (self.azimuths[pos] - azimuth_degrees).abs();
            if dist_prev <= dist_curr {
                Some(pos - 1)
            } else {
                Some(pos)
            }
        }
    }

    /// Extract gate parameters from the first radial that has data for this product.
    fn extract_gate_params(radials: &[Radial], product: Product) -> Option<(f64, f64, usize)> {
        for radial in radials {
            if let Some(moment) = product.moment_data(radial) {
                return Some((
                    moment.first_gate_range_km(),
                    moment.gate_interval_km(),
                    moment.gate_count() as usize,
                ));
            }
            if let Some(cfp) = product.cfp_moment_data(radial) {
                return Some((
                    cfp.first_gate_range_km(),
                    cfp.gate_interval_km(),
                    cfp.gate_count() as usize,
                ));
            }
        }
        None
    }

    /// Check whether a radial has data for the given product.
    fn radial_has_product(radial: &Radial, product: Product) -> bool {
        product.moment_data(radial).is_some() || product.cfp_moment_data(radial).is_some()
    }

    /// Decode moment data from a radial directly into pre-allocated output slices.
    ///
    /// The slices must be exactly `gate_count` long. Gates beyond what the moment
    /// provides are left at their initial values (0.0 / NoData).
    fn decode_moment_into(
        radial: &Radial,
        product: Product,
        values: &mut [f32],
        status: &mut [GateStatus],
    ) {
        if let Some(moment) = product.moment_data(radial) {
            for (i, mv) in moment.iter().enumerate() {
                if i >= values.len() {
                    break;
                }
                match mv {
                    MomentValue::Value(v) => {
                        values[i] = v;
                        status[i] = GateStatus::Valid;
                    }
                    MomentValue::BelowThreshold => {
                        status[i] = GateStatus::BelowThreshold;
                    }
                    MomentValue::RangeFolded => {
                        status[i] = GateStatus::RangeFolded;
                    }
                }
            }
            return;
        }

        if let Some(cfp) = product.cfp_moment_data(radial) {
            for (i, cv) in cfp.iter().enumerate() {
                if i >= values.len() {
                    break;
                }
                match cv {
                    CFPMomentValue::Value(v) => {
                        values[i] = v;
                        status[i] = GateStatus::Valid;
                    }
                    CFPMomentValue::Status(_) => {
                        // values[i] stays 0.0, status stays NoData (initial values)
                    }
                }
            }
        }
    }
}

/// A 2D Cartesian grid of values on a geographic extent.
///
/// Used for scan-derived products like composite reflectivity, echo tops, and VIL
/// where data from multiple elevations is combined into a single geographic surface.
///
/// Layout: `values[row * width + col]`, row 0 = north edge.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CartesianField {
    /// Human-readable label.
    label: String,
    /// Physical unit string.
    unit: String,
    /// Geographic extent of the grid.
    extent: GeoExtent,
    /// Number of columns.
    width: usize,
    /// Number of rows.
    height: usize,
    /// Flat array of values, row-major.
    values: Vec<f32>,
    /// Parallel status array.
    status: Vec<GateStatus>,
}

impl CartesianField {
    /// Create a new empty Cartesian field.
    ///
    /// All values are initialized to `0.0` with [`GateStatus::NoData`].
    pub fn new(
        label: impl Into<String>,
        unit: impl Into<String>,
        extent: GeoExtent,
        width: usize,
        height: usize,
    ) -> Self {
        let total = width * height;
        Self {
            label: label.into(),
            unit: unit.into(),
            extent,
            width,
            height,
            values: vec![0.0; total],
            status: vec![GateStatus::NoData; total],
        }
    }

    /// Human-readable label for this field.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Physical unit string.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// Geographic extent of the grid.
    pub fn extent(&self) -> &GeoExtent {
        &self.extent
    }

    /// Number of columns.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Number of rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// All values as a flat slice.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// All statuses as a flat slice.
    pub fn statuses(&self) -> &[GateStatus] {
        &self.status
    }

    /// Get the value and status at a specific (row, col).
    pub fn get(&self, row: usize, col: usize) -> (f32, GateStatus) {
        let idx = row * self.width + col;
        (self.values[idx], self.status[idx])
    }

    /// Set the value and status at a specific (row, col).
    pub fn set(&mut self, row: usize, col: usize, value: f32, status: GateStatus) {
        let idx = row * self.width + col;
        self.values[idx] = value;
        self.status[idx] = status;
    }

    /// Query the value at a geographic point.
    ///
    /// Returns `None` if the point is outside the field's extent.
    pub fn value_at_geo(&self, point: GeoPoint) -> Option<(f32, GateStatus)> {
        if point.latitude < self.extent.min.latitude
            || point.latitude > self.extent.max.latitude
            || point.longitude < self.extent.min.longitude
            || point.longitude > self.extent.max.longitude
        {
            return None;
        }

        let lat_frac = (self.extent.max.latitude - point.latitude)
            / (self.extent.max.latitude - self.extent.min.latitude);
        let lon_frac = (point.longitude - self.extent.min.longitude)
            / (self.extent.max.longitude - self.extent.min.longitude);

        let row = (lat_frac * self.height as f64) as usize;
        let col = (lon_frac * self.width as f64) as usize;

        let row = row.min(self.height - 1);
        let col = col.min(self.width - 1);

        Some(self.get(row, col))
    }

    /// Compute the (min, max) of valid values in this field.
    pub fn value_range(&self) -> Option<(f32, f32)> {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        let mut found = false;

        for (i, &val) in self.values.iter().enumerate() {
            if self.status[i] == GateStatus::Valid {
                min = min.min(val);
                max = max.max(val);
                found = true;
            }
        }

        if found {
            Some((min, max))
        } else {
            None
        }
    }
}

/// A 2D vertical cross-section grid of values.
///
/// Used for RHI (Range-Height Indicator) displays and arbitrary vertical slices through
/// a scan. The horizontal axis represents distance from the radar (or from the
/// start of a cross-section path) and the vertical axis represents altitude.
///
/// Layout: `values[row * width + col]`, row 0 = top (highest altitude).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct VerticalField {
    /// Human-readable label.
    label: String,
    /// Physical unit string.
    unit: String,
    /// Horizontal axis: (min_km, max_km) distance range.
    distance_range_km: (f64, f64),
    /// Vertical axis: (min_meters, max_meters) altitude range.
    altitude_range_meters: (f64, f64),
    /// Number of horizontal bins (columns).
    width: usize,
    /// Number of vertical bins (rows).
    height: usize,
    /// Flat array of values, row-major.
    values: Vec<f32>,
    /// Parallel status array.
    status: Vec<GateStatus>,
}

impl VerticalField {
    /// Create a new empty vertical field.
    ///
    /// All values are initialized to `0.0` with [`GateStatus::NoData`].
    pub fn new(
        label: impl Into<String>,
        unit: impl Into<String>,
        distance_range_km: (f64, f64),
        altitude_range_meters: (f64, f64),
        width: usize,
        height: usize,
    ) -> Self {
        let total = width * height;
        Self {
            label: label.into(),
            unit: unit.into(),
            distance_range_km,
            altitude_range_meters,
            width,
            height,
            values: vec![0.0; total],
            status: vec![GateStatus::NoData; total],
        }
    }

    /// Human-readable label for this field.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Physical unit string.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// Horizontal axis: (min_km, max_km) distance range.
    pub fn distance_range_km(&self) -> (f64, f64) {
        self.distance_range_km
    }

    /// Vertical axis: (min_meters, max_meters) altitude range.
    pub fn altitude_range_meters(&self) -> (f64, f64) {
        self.altitude_range_meters
    }

    /// Number of horizontal bins (columns).
    pub fn width(&self) -> usize {
        self.width
    }

    /// Number of vertical bins (rows).
    pub fn height(&self) -> usize {
        self.height
    }

    /// All values as a flat slice.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// All statuses as a flat slice.
    pub fn statuses(&self) -> &[GateStatus] {
        &self.status
    }

    /// Get the value and status at a specific (row, col).
    pub fn get(&self, row: usize, col: usize) -> (f32, GateStatus) {
        let idx = row * self.width + col;
        (self.values[idx], self.status[idx])
    }

    /// Set the value and status at a specific (row, col).
    pub fn set(&mut self, row: usize, col: usize, value: f32, status: GateStatus) {
        let idx = row * self.width + col;
        self.values[idx] = value;
        self.status[idx] = status;
    }

    /// Query the value at a distance/altitude coordinate.
    ///
    /// Returns `None` if the coordinate is outside the field's range.
    pub fn value_at(&self, distance_km: f64, altitude_meters: f64) -> Option<(f32, GateStatus)> {
        let (d_min, d_max) = self.distance_range_km;
        let (a_min, a_max) = self.altitude_range_meters;

        if distance_km < d_min
            || distance_km > d_max
            || altitude_meters < a_min
            || altitude_meters > a_max
        {
            return None;
        }

        let col_frac = (distance_km - d_min) / (d_max - d_min);
        let row_frac = (a_max - altitude_meters) / (a_max - a_min); // Inverted: top = highest

        let col = (col_frac * self.width as f64) as usize;
        let row = (row_frac * self.height as f64) as usize;

        let col = col.min(self.width - 1);
        let row = row.min(self.height - 1);

        Some(self.get(row, col))
    }

    /// Compute the (min, max) of valid values in this field.
    pub fn value_range(&self) -> Option<(f32, f32)> {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        let mut found = false;

        for (i, &val) in self.values.iter().enumerate() {
            if self.status[i] == GateStatus::Valid {
                min = min.min(val);
                max = max.max(val);
                found = true;
            }
        }

        if found {
            Some((min, max))
        } else {
            None
        }
    }
}
