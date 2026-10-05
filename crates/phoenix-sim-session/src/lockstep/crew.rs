use phoenix_model::messages::StationId;

pub const MAX_FLEET_CREW_SEATS: usize = 128;

pub const MAX_FLEET_CREW_FIELD_BYTES: usize = 128;

pub fn canonical_station_ratings(
    mut crew: Vec<(StationId, String)>,
) -> Option<Vec<(StationId, String)>> {
    let valid = |text: &str| {
        !text.is_empty()
            && text.len() <= MAX_FLEET_CREW_FIELD_BYTES
            && !text.chars().any(char::is_control)
    };
    if crew.len() > MAX_FLEET_CREW_SEATS
        || crew
            .iter()
            .any(|(station, rating)| !valid(&station.0) || !valid(rating))
    {
        return None;
    }
    crew.sort_by(|a, b| a.0 .0.cmp(&b.0 .0));
    if crew.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    Some(crew)
}
