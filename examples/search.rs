// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::io::{BufRead, stdin};

use egg_mode::search::{self, ResultType};

mod common;

fn main() {
    let config = common::Config::load();

    println!("Search term:");
    let line = stdin().lock().lines().next().unwrap().unwrap();

    let search = block_on_all(
        search::search(line)
            .result_type(ResultType::Recent)
            .count(10)
            .call(&config.token),
    )
    .unwrap();

    for tweet in &search.statuses {
        common::print_tweet(tweet);
        println!()
    }
}
