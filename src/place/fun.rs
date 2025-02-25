// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use futures_core::Future;

use crate::{auth, error, links};
use crate::common::*;
use crate::error::Error::BadUrl;

use super::*;
use super::PlaceQuery;

/// Load the place with the given ID.
///
/// ## Examples
///
/// ```rust,no_run
/// # use egg_mode::Token;
/// use tokio::runtime::current_thread::block_on_all;
/// # fn main() {
/// # let token: Token = unimplemented!();
/// let result = block_on_all(egg_mode::place::show("18810aa5b43e76c7", &token)).unwrap();
///
/// assert!(result.full_name == "Dallas, TX");
/// # }
/// ```
pub fn show(id: &str, token: &auth::Token) -> FutureResponse<Place> {
    let url = format!("{}/{}.json", links::place::SHOW_STEM, id);

    let req = auth::get(&url, token, None);

    make_parsed_future(req)
}

/// Begins building a reverse-geocode search with the given coordinate.
///
/// ## Examples
///
/// ```rust,no_run
/// # use egg_mode::Token;
/// use tokio::runtime::current_thread::block_on_all;
/// # fn main() {
/// # let token: Token = unimplemented!();
/// use egg_mode::place::{self, PlaceType};
/// let result = block_on_all(place::reverse_geocode(51.507222, -0.1275)
///                                 .granularity(PlaceType::City)
///                                 .call(&token))
///                  .unwrap();
///
/// assert!(result.results.iter().any(|pl| pl.full_name == "London, England"));
/// # }
/// ```
pub fn reverse_geocode(latitude: f64, longitude: f64) -> GeocodeBuilder {
    GeocodeBuilder::new(latitude, longitude)
}

fn parse_url<'a>(base: &'static str, full: &'a str) -> Result<ParamList<'a>, error::Error> {
    let mut iter = full.split('?');

    if let Some(base_part) = iter.next() {
        if base_part != base {
            return Err(BadUrl);
        }
    } else {
        return Err(BadUrl);
    }

    if let Some(list) = iter.next() {
        let mut p = HashMap::new();

        for item in list.split('&') {
            let mut kv_iter = item.split('=');

            let k = kv_iter.next().ok_or(BadUrl)?;
            let v = kv_iter.next().ok_or(BadUrl)?;

            add_param(&mut p, k, v);
        }

        Ok(p)
    } else {
        Err(BadUrl)
    }
}

///From a URL given with the result of `reverse_geocode`, perform the same reverse-geocode search.
///
///## Errors
///
///In addition to errors that might occur generally, this function will return a `BadUrl` error if
///the given URL is not a valid `reverse_geocode` query URL.
pub fn reverse_geocode_url<'a>(
    url: &'a str,
    token: &'a auth::Token,
) -> impl Future<Output = Result<Response<SearchResult>, error::Error>> + 'a {
    // TODO handle error
    let params = parse_url(links::place::REVERSE_GEOCODE, url).unwrap();
    let req = auth::get(links::place::REVERSE_GEOCODE, &token, Some(&params));
    make_parsed_future(req)
}

/// Begins building a location search via latitude/longitude.
///
/// ## Example
///
/// ```rust,no_run
/// # use egg_mode::Token;
/// use tokio::runtime::current_thread::block_on_all;
/// # fn main() {
/// # let token: Token = unimplemented!();
/// use egg_mode::place::{self, PlaceType};
/// let result = block_on_all(place::search_point(51.507222, -0.1275)
///                                 .granularity(PlaceType::City)
///                                 .call(&token))
///                  .unwrap();
///
/// assert!(result.results.iter().any(|pl| pl.full_name == "London, England"));
/// # }
/// ```
pub fn search_point(latitude: f64, longitude: f64) -> SearchBuilder<'static> {
    SearchBuilder::new(PlaceQuery::LatLon(latitude, longitude))
}

/// Begins building a location search via a text query.
///
/// ## Example
///
/// ```rust,no_run
/// # use egg_mode::Token;
/// use tokio::runtime::current_thread::block_on_all;
/// # fn main() {
/// # let token: Token = unimplemented!();
/// use egg_mode::place::{self, PlaceType};
/// let result = block_on_all(place::search_query("columbia")
///                                 .granularity(PlaceType::Admin)
///                                 .call(&token))
///                  .unwrap();
///
/// assert!(result.results.iter().any(|pl| pl.full_name == "British Columbia, Canada"));
/// # }
/// ```
pub fn search_query(query: &str) -> SearchBuilder {
    SearchBuilder::new(PlaceQuery::Query(query))
}

///Begins building a location search via an IP address.
pub fn search_ip(query: &str) -> SearchBuilder {
    SearchBuilder::new(PlaceQuery::IPAddress(query))
}

///From a URL given with the result of any `search_*` function, perform the same location search.
///
///## Errors
///
///In addition to errors that might occur generally, this function will return a `BadUrl` error if
///the given URL is not a valid `search` query URL.
pub fn search_url<'a>(
    url: &'a str,
    token: &'a auth::Token,
) -> impl Future<Output = Result<Response<SearchResult>, error::Error>> + 'a {
    let params = parse_url(links::place::SEARCH, url).unwrap();
    let req = auth::get(links::place::REVERSE_GEOCODE, &token, Some(&params));
    make_parsed_future(req)
}
