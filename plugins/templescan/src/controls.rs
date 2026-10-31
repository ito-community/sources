//! The browse controls: the site's Status, Order by and direction selectors.

use crate::catalog::{Browse, Direction, Order};
use ito_rs::Result;
use ito_rs::controls::{
    ChoiceOption, Control, ControlSection, ControlValues, OptionSource, SearchCapabilities,
};
use ito_rs::models::FilterItem;

pub const ORDER: &str = "order";
pub const STATUS: &str = "status";
pub const DIRECTION: &str = "direction";

/// The site's statuses, spelled as it spells them.
pub const STATUSES: [&str; 5] = ["Ongoing", "Hiatus", "Completed", "Canceled", "Dropped"];

fn inline(pairs: &[(&str, &str)]) -> OptionSource {
    OptionSource::inline(
        pairs
            .iter()
            .map(|(value, label)| ChoiceOption::new(*value, *label))
            .collect(),
    )
}

/// Browsing is the empty search: the controls pick what the catalog lists and how it is
/// ordered. The site's text search takes neither, so they apply only without a query.
pub fn capabilities() -> SearchCapabilities {
    let status: Vec<(&str, &str)> = STATUSES.iter().map(|status| (*status, *status)).collect();
    SearchCapabilities {
        flags: SearchCapabilities::BROWSE,
        sort: Some(Control::select(
            ORDER,
            "Order by",
            inline(&[
                ("update_chapter", "Update Chapter"),
                ("created_at", "Created At"),
                ("title", "Title"),
                ("total_views", "Trending"),
            ]),
            Some("update_chapter".into()),
        )),
        sections: vec![
            ControlSection::new("filters", "Filters")
                .control(Control::select(STATUS, "Status", inline(&status), None))
                .control(Control::select(
                    DIRECTION,
                    "Direction",
                    inline(&[("desc", "Descending"), ("asc", "Ascending")]),
                    Some("desc".into()),
                )),
        ],
    }
}

/// What a browse asks for, from the values a host sends. Hosts without rich controls send none,
/// which browses newest update first across every status. A saved choice the site no longer
/// offers keeps its meaning: an unknown status lists nothing, while an unknown order or
/// direction falls back to the default.
pub fn browse(filters: &[FilterItem]) -> Result<Browse> {
    if filters.is_empty() {
        return Ok(Browse::default());
    }
    let values = ControlValues::search(&capabilities(), filters)?;
    let defaults = Browse::default();
    Ok(Browse {
        status: values.select(STATUS)?.map(str::to_owned),
        order: values
            .select(ORDER)?
            .and_then(Order::from_id)
            .unwrap_or(defaults.order),
        direction: values
            .select(DIRECTION)?
            .and_then(Direction::from_id)
            .unwrap_or(defaults.direction),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ito_rs::controls::{ControlValue, validate_capabilities};
    use ito_rs::models::FilterValue;
    use std::collections::HashMap;

    fn dense(values: &[(&str, ControlValue)]) -> Vec<FilterItem> {
        let capabilities = capabilities();
        let map: HashMap<String, ControlValue> = values
            .iter()
            .map(|(id, value)| ((*id).to_owned(), value.clone()))
            .collect();
        capabilities.filter_items(&map)
    }

    fn select(value: &str) -> ControlValue {
        ControlValue::Select(Some(value.to_owned()))
    }

    #[test]
    fn the_declared_controls_are_valid_for_every_host() {
        validate_capabilities(&capabilities()).expect("valid capabilities");
        let capabilities = capabilities();
        assert_eq!(capabilities.flags, SearchCapabilities::BROWSE);
        let ids: Vec<String> = capabilities.controls().into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["order", "status", "direction"]);
    }

    #[test]
    fn defaults_browse_by_newest_update_across_every_status() {
        assert_eq!(browse(&[]).expect("browse"), Browse::default());
        // A host that sends every control at its default means the same.
        assert_eq!(browse(&dense(&[])).expect("browse"), Browse::default());
    }

    #[test]
    fn chosen_values_reach_the_browse() {
        let filters = dense(&[
            (ORDER, select("total_views")),
            (STATUS, select("Completed")),
            (DIRECTION, select("asc")),
        ]);
        let chosen = browse(&filters).expect("browse");
        assert_eq!(chosen.status.as_deref(), Some("Completed"));
        assert_eq!(chosen.order, Order::Views);
        assert_eq!(chosen.direction, Direction::Ascending);
        for (value, order) in [
            ("update_chapter", Order::Updated),
            ("created_at", Order::Created),
            ("title", Order::Title),
            ("total_views", Order::Views),
        ] {
            let filters = dense(&[(ORDER, select(value))]);
            assert_eq!(browse(&filters).expect("browse").order, order);
        }
    }

    #[test]
    fn choices_the_site_no_longer_offers_keep_their_meaning() {
        let filters = dense(&[
            (ORDER, select("popular")),
            (STATUS, select("Cancelled")),
            (DIRECTION, select("sideways")),
        ]);
        let chosen = browse(&filters).expect("browse");
        assert_eq!(chosen.order, Order::Updated);
        assert_eq!(chosen.direction, Direction::Descending);
        assert_eq!(chosen.status.as_deref(), Some("Cancelled"));
    }

    #[test]
    fn malformed_filters_are_errors_not_defaults() {
        let wrong_kind = vec![FilterItem {
            type_name: "toggle".into(),
            name: ORDER.into(),
            value: FilterValue::String("true".into()),
        }];
        assert!(browse(&wrong_kind).is_err());
        let not_text = vec![FilterItem {
            type_name: "select".into(),
            name: STATUS.into(),
            value: FilterValue::Boolean(true),
        }];
        assert!(browse(&not_text).is_err());
        let duplicate = vec![
            FilterItem {
                type_name: "select".into(),
                name: STATUS.into(),
                value: FilterValue::String("Ongoing".into()),
            },
            FilterItem {
                type_name: "select".into(),
                name: STATUS.into(),
                value: FilterValue::String("Dropped".into()),
            },
        ];
        assert!(browse(&duplicate).is_err());
    }
}
