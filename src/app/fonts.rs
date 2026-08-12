use eframe::egui;

static FONT_ALEGREYA: &[u8] =
    include_bytes!("../../assets/fonts/Alegreya/Alegreya-VariableFont_wght.ttf");
static FONT_ATKINSON: &[u8] =
    include_bytes!("../../assets/fonts/Atkinson_Hyperlegible/AtkinsonHyperlegible-Regular.ttf");
static FONT_EB_GARAMOND: &[u8] =
    include_bytes!("../../assets/fonts/EB_Garamond/EBGaramond-VariableFont_wght.ttf");
static FONT_INTER: &[u8] =
    include_bytes!("../../assets/fonts/Inter/Inter-VariableFont_opsz,wght.ttf");
static FONT_LATO: &[u8] = include_bytes!("../../assets/fonts/Lato/Lato-Regular.ttf");
static FONT_LITERATA: &[u8] =
    include_bytes!("../../assets/fonts/Literata/Literata-VariableFont_opsz,wght.ttf");
static FONT_MERRIWEATHER: &[u8] =
    include_bytes!("../../assets/fonts/Merriweather/Merriweather-VariableFont_opsz,wdth,wght.ttf");
static FONT_NUNITO: &[u8] =
    include_bytes!("../../assets/fonts/Nunito/Nunito-VariableFont_wght.ttf");
static FONT_POLTAWSKI: &[u8] =
    include_bytes!("../../assets/fonts/Poltawski_Nowy/PoltawskiNowy-VariableFont_wght.ttf");
static FONT_SOURCE_SANS: &[u8] =
    include_bytes!("../../assets/fonts/Source_Sans_3/SourceSans3-VariableFont_wght.ttf");
static FONT_VOLLKORN: &[u8] =
    include_bytes!("../../assets/fonts/Vollkorn/Vollkorn-VariableFont_wght.ttf");

pub fn font_families() -> Vec<&'static str> {
    vec![
        "Alegreya",
        "Atkinson Hyperlegible",
        "EB Garamond",
        "Inter",
        "Lato",
        "Literata",
        "Merriweather",
        "Nunito",
        "Poltawski Nowy",
        "Source Sans 3",
        "Vollkorn",
    ]
}

fn font_data(family: &str) -> Option<&'static [u8]> {
    match family {
        "Alegreya" => Some(FONT_ALEGREYA),
        "Atkinson Hyperlegible" => Some(FONT_ATKINSON),
        "EB Garamond" => Some(FONT_EB_GARAMOND),
        "Inter" => Some(FONT_INTER),
        "Lato" => Some(FONT_LATO),
        "Literata" => Some(FONT_LITERATA),
        "Merriweather" => Some(FONT_MERRIWEATHER),
        "Nunito" => Some(FONT_NUNITO),
        "Poltawski Nowy" => Some(FONT_POLTAWSKI),
        "Source Sans 3" => Some(FONT_SOURCE_SANS),
        "Vollkorn" => Some(FONT_VOLLKORN),
        _ => None,
    }
}

pub fn load_font(ctx: &egui::Context, family: &str) -> bool {
    let loaded = font_data(family).map(|data| {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "reader_font".to_owned(),
            egui::FontData::from_owned(data.to_vec()).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Name("reader_font".into()))
            .or_default()
            .push("reader_font".to_owned());
        fonts
    });
    if let Some(fonts) = loaded {
        ctx.set_fonts(fonts);
        true
    } else {
        false
    }
}

pub fn reader_font_family() -> egui::FontFamily {
    egui::FontFamily::Name("reader_font".into())
}
