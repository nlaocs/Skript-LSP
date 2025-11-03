mod internal_utils;
pub mod types;

use types::AbstractAddonSyntaxList;
pub fn fetch_data() -> Result<AbstractAddonSyntaxList, reqwest::Error> {
    const URL: &str = "https://skripthub.net/api/v1/addonsyntaxlist/";
    // この関数自体実行されるのは最初の一度限りなので、blockingで良い
    let resp: AbstractAddonSyntaxList = reqwest::blocking::get(URL)?.json()?;
    Ok(resp)
}
