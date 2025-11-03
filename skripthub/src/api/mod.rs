mod internal_utils;
pub mod types;

use types::AbstractAddonSyntaxList;
pub fn fetch_data() -> Result<AbstractAddonSyntaxList, ureq::Error> {
    const URL: &str = "https://skripthub.net/api/v1/addonsyntaxlist/";
    // この関数自体実行されるのは最初の一度限りなので、blockingで良い
    let resp = ureq::get(URL)
        .call()?
        .body_mut()
        .read_json::<AbstractAddonSyntaxList>()?;
    Ok(resp)
}
