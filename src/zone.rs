//! # Zone
//!
//! Transport analysis zone with socioeconomic attributes
//! used by the trip generation step.

use crate::gmns::types::ZoneID;

/// A transport analysis zone (TAZ).
#[derive(Debug, Clone)]
pub struct Zone {
    /// Unique zone identifier.
    pub id: ZoneID,
    /// Zone name or description.
    pub name: String,
    /// Total population.
    pub population: f64,
    /// Total employment (number of jobs).
    pub employment: f64,
    /// Area in square kilometers.
    pub area_sq_km: f64,
    /// Average household income.
    pub avg_income: f64,
    /// Number of households.
    pub households: f64,
}

impl Zone {
    /// Create a new builder with the required zone ID.
    ///
    /// # Arguments
    /// * `id` - Unique zone identifier.
    ///
    /// # Returns
    /// A `ZoneBuilder` instance for method chaining.
    ///
    /// # Example
    /// ```
    /// use macro_traffic_sim_core::zone::Zone;
    ///
    /// let zone = Zone::new(1)
    ///     .with_population(50000.0)
    ///     .with_employment(20000.0)
    ///     .build();
    /// ```
    pub fn new(id: ZoneID) -> ZoneBuilder {
        ZoneBuilder {
            instance: Zone {
                id,
                name: String::new(),
                population: 0.0,
                employment: 0.0,
                area_sq_km: 0.0,
                avg_income: 0.0,
                households: 0.0,
            },
        }
    }
}

/// Builder for `Zone`.
pub struct ZoneBuilder {
    instance: Zone,
}

impl ZoneBuilder {
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.instance.name = name.into();
        self
    }

    pub fn with_population(mut self, population: f64) -> Self {
        self.instance.population = population;
        self
    }

    pub fn with_employment(mut self, employment: f64) -> Self {
        self.instance.employment = employment;
        self
    }

    pub fn with_area_sq_km(mut self, area: f64) -> Self {
        self.instance.area_sq_km = area;
        self
    }

    pub fn with_avg_income(mut self, income: f64) -> Self {
        self.instance.avg_income = income;
        self
    }

    pub fn with_households(mut self, households: f64) -> Self {
        self.instance.households = households;
        self
    }

    /// Construct the final `Zone`.
    pub fn build(self) -> Zone {
        self.instance
    }
}
