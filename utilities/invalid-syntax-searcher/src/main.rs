use syntaxes::Syntaxes;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let syntaxes = Syntaxes::initialize();
    Ok(())
}
