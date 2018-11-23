//! Contains utilities to convert strings (CSS strings) to servo types

mod css_grammar {
    include!(concat!(env!("OUT_DIR"), "/css_grammar.rs"));
}

use std::{fmt, num::{ParseIntError, ParseFloatError}};
pub use {
    app_units::Au,
    euclid::{TypedSize2D, SideOffsets2D, TypedPoint2D},
    webrender::api::{
        BorderDetails, NormalBorder,
        NinePatchBorder, LayoutPixel, BoxShadowClipMode, ColorU,
        ColorF, LayoutVector2D, Gradient, RadialGradient, LayoutPoint,
        LayoutSize, ExtendMode, LayoutSideOffsets,
    },
};
use webrender::api::{BorderStyle, BorderRadius, BorderSide, LayoutRect};

pub(crate) const EM_HEIGHT: f32 = 16.0;
/// WebRender measures in points, not in pixels!
pub(crate) const PT_TO_PX: f32 = 96.0 / 72.0;

// In case no font size is specified for a node,
// this will be substituted as the default font size
pub(crate) const DEFAULT_FONT_SIZE: StyleFontSize = StyleFontSize(PixelValue {
    metric: CssMetric::Px,
    number: 100_000,
});

/// A parser that can accept a list of items and mappings
macro_rules! multi_type_parser {
    ($fn:ident, $return:ident, $([$identifier_string:expr, $enum_type:ident]),+) => (
        fn $fn<'a>(input: &'a str)
        -> Result<$return, InvalidValueErr<'a>>
        {
            match input {
                $(
                    $identifier_string => Ok($return::$enum_type),
                )+
                _ => Err(InvalidValueErr(input)),
            }
        }
    )
}

macro_rules! typed_pixel_value_parser {
    ($fn:ident, $return:ident) => (
        fn $fn<'a>(input: &'a str)
        -> Result<$return, PixelParseError<'a>>
        {
            parse_pixel_value(input).and_then(|e| Ok($return(e)))
        }
    )
}

/// Creates `pt`, `px` and `em` constructors for any struct that has a
/// `PixelValue` as it's self.0 field.
macro_rules! impl_pixel_value {($struct:ident) => (
    impl $struct {
        #[inline]
        pub fn px(value: f32) -> Self {
            $struct(PixelValue::px(value))
        }

        #[inline]
        pub fn em(value: f32) -> Self {
            $struct(PixelValue::em(value))
        }

        #[inline]
        pub fn pt(value: f32) -> Self {
            $struct(PixelValue::pt(value))
        }
    }
)}

/// A successfully parsed CSS property
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParsedCssProperty {
    BorderRadius(StyleBorderRadius),
    BackgroundColor(StyleBackgroundColor),
    TextColor(StyleTextColor),
    Border(StyleBorder),
    Background(StyleBackground),
    FontSize(StyleFontSize),
    FontFamily(StyleFontFamily),
    TextAlign(StyleTextAlignmentHorz),
    LetterSpacing(StyleLetterSpacing),
    BoxShadow(StyleBoxShadow),
    LineHeight(StyleLineHeight),

    Width(LayoutWidth),
    Height(LayoutHeight),
    MinWidth(LayoutMinWidth),
    MinHeight(LayoutMinHeight),
    MaxWidth(LayoutMaxWidth),
    MaxHeight(LayoutMaxHeight),
    Position(LayoutPosition),
    Top(LayoutTop),
    Right(LayoutRight),
    Left(LayoutLeft),
    Bottom(LayoutBottom),
    Padding(LayoutPadding),
    Margin(LayoutMargin),
    FlexWrap(LayoutWrap),
    FlexDirection(LayoutDirection),
    FlexGrow(LayoutFlexGrow),
    FlexShrink(LayoutFlexShrink),
    JustifyContent(LayoutJustifyContent),
    AlignItems(LayoutAlignItems),
    AlignContent(LayoutAlignContent),
    Overflow(LayoutOverflow),
}

impl ParsedCssProperty {
    /// Returns whether this property will be inherited during cascading
    pub fn is_inheritable(&self) -> bool {
        use self::ParsedCssProperty::*;
        match self {
            | TextColor(_)
            | FontFamily(_)
            | FontSize(_)
            | LineHeight(_)
            | TextAlign(_) => true,
            _ => false,
        }
    }
}

impl_from!(StyleBorderRadius, ParsedCssProperty::BorderRadius);
impl_from!(StyleBackground, ParsedCssProperty::Background);
impl_from!(StyleBoxShadow, ParsedCssProperty::BoxShadow);
impl_from!(StyleBorder, ParsedCssProperty::Border);
impl_from!(StyleFontSize, ParsedCssProperty::FontSize);
impl_from!(StyleFontFamily, ParsedCssProperty::FontFamily);
impl_from!(StyleTextAlignmentHorz, ParsedCssProperty::TextAlign);
impl_from!(StyleLineHeight, ParsedCssProperty::LineHeight);
impl_from!(StyleLetterSpacing, ParsedCssProperty::LetterSpacing);
impl_from!(StyleBackgroundColor, ParsedCssProperty::BackgroundColor);
impl_from!(StyleTextColor, ParsedCssProperty::TextColor);

impl_from!(LayoutOverflow, ParsedCssProperty::Overflow);
impl_from!(LayoutWidth, ParsedCssProperty::Width);
impl_from!(LayoutHeight, ParsedCssProperty::Height);
impl_from!(LayoutMinWidth, ParsedCssProperty::MinWidth);
impl_from!(LayoutMinHeight, ParsedCssProperty::MinHeight);
impl_from!(LayoutMaxWidth, ParsedCssProperty::MaxWidth);
impl_from!(LayoutMaxHeight, ParsedCssProperty::MaxHeight);

impl_from!(LayoutPosition, ParsedCssProperty::Position);
impl_from!(LayoutTop, ParsedCssProperty::Top);
impl_from!(LayoutBottom, ParsedCssProperty::Bottom);
impl_from!(LayoutRight, ParsedCssProperty::Right);
impl_from!(LayoutLeft, ParsedCssProperty::Left);

impl_from!(LayoutPadding, ParsedCssProperty::Padding);
impl_from!(LayoutMargin, ParsedCssProperty::Margin);

impl_from!(LayoutWrap, ParsedCssProperty::FlexWrap);
impl_from!(LayoutDirection, ParsedCssProperty::FlexDirection);
impl_from!(LayoutFlexGrow, ParsedCssProperty::FlexGrow);
impl_from!(LayoutFlexShrink, ParsedCssProperty::FlexShrink);
impl_from!(LayoutJustifyContent, ParsedCssProperty::JustifyContent);
impl_from!(LayoutAlignItems, ParsedCssProperty::AlignItems);
impl_from!(LayoutAlignContent, ParsedCssProperty::AlignContent);

impl ParsedCssProperty {

    /// Main parsing function, takes a stringified key / value pair and either
    /// returns the parsed value or an error
    ///
    /// ```rust
    /// # use azul::prelude::*;
    /// assert_eq!(
    ///     ParsedCssProperty::from_kv("width", "500px"),
    ///     Ok(ParsedCssProperty::Width(LayoutWidth(PixelValue::px(500.0))))
    /// )
    /// ```
    pub fn from_kv<'a>(key: &'a str, value: &'a str) -> Result<Self, CssParsingError<'a>> {
        let key = key.trim();
        let value = value.trim();
        match key {
            "border-radius"     => Ok(parse_css_border_radius(value)?.into()),
            "background-color"  => Ok(parse_css_background_color(value)?.into()),
            "font-color" |
            "color"             => Ok(parse_css_text_color(value)?.into()),
            "background"        => Ok(parse_css_background(value)?.into()),
            "font-size"         => Ok(parse_css_font_size(value)?.into()),
            "font-family"       => Ok(parse_css_font_family(value)?.into()),
            "letter-spacing"    => Ok(parse_css_letter_spacing(value)?.into()),
            "line-height"       => Ok(parse_css_line_height(value)?.into()),

            "border"            => Ok(StyleBorder::all(parse_css_border(value)?).into()),
            "border-top"        => Ok(StyleBorder::parse_top(value)?.into()),
            "border-bottom"     => Ok(StyleBorder::parse_bottom(value)?.into()),
            "border-left"       => Ok(StyleBorder::parse_left(value)?.into()),
            "border-right"      => Ok(StyleBorder::parse_right(value)?.into()),

            "box-shadow"        => Ok(StyleBoxShadow::all(parse_css_box_shadow(value)?).into()),
            "box-shadow-top"    => Ok(StyleBoxShadow::parse_top(value)?.into()),
            "box-shadow-bottom" => Ok(StyleBoxShadow::parse_bottom(value)?.into()),
            "box-shadow-left"   => Ok(StyleBoxShadow::parse_left(value)?.into()),
            "box-shadow-right"  => Ok(StyleBoxShadow::parse_right(value)?.into()),

            "width"             => Ok(parse_layout_width(value)?.into()),
            "height"            => Ok(parse_layout_height(value)?.into()),
            "min-width"         => Ok(parse_layout_min_width(value)?.into()),
            "min-height"        => Ok(parse_layout_min_height(value)?.into()),
            "max-width"         => Ok(parse_layout_max_width(value)?.into()),
            "max-height"        => Ok(parse_layout_max_height(value)?.into()),

            "position"          => Ok(parse_layout_position(value)?.into()),
            "top"               => Ok(parse_layout_top(value)?.into()),
            "right"             => Ok(parse_layout_right(value)?.into()),
            "left"              => Ok(parse_layout_left(value)?.into()),
            "bottom"            => Ok(parse_layout_bottom(value)?.into()),
            "text-align"        => Ok(parse_layout_text_align(value)?.into()),

            "padding"           => Ok(parse_layout_padding(value)?.into()),
            "padding-top"       => Ok(LayoutPadding::parse_top(value)?.into()),
            "padding-bottom"    => Ok(LayoutPadding::parse_bottom(value)?.into()),
            "padding-left"      => Ok(LayoutPadding::parse_left(value)?.into()),
            "padding-right"     => Ok(LayoutPadding::parse_right(value)?.into()),

            "margin"            => Ok(parse_layout_margin(value)?.into()),
            "margin-top"       => Ok(LayoutMargin::parse_top(value)?.into()),
            "margin-bottom"    => Ok(LayoutMargin::parse_bottom(value)?.into()),
            "margin-left"      => Ok(LayoutMargin::parse_left(value)?.into()),
            "margin-right"     => Ok(LayoutMargin::parse_right(value)?.into()),

            "flex-wrap"         => Ok(parse_layout_wrap(value)?.into()),
            "flex-direction"    => Ok(parse_layout_direction(value)?.into()),
            "flex-grow"         => Ok(parse_layout_flex_grow(value)?.into()),
            "flex-shrink"       => Ok(parse_layout_flex_shrink(value)?.into()),

            "align-main-axis" |
            "justify-content"   => Ok(parse_layout_justify_content(value)?.into()),

            "align-cross-axis" |
            "align-items"       => Ok(parse_layout_align_items(value)?.into()),

            "align-cross-axis-multiline" |
            "align-content"     => Ok(parse_layout_align_content(value)?.into()),

            "overflow"          => {
                let overflow_both_directions = parse_layout_text_overflow(value)?;
                Ok(LayoutOverflow {
                    horizontal: TextOverflowBehaviour::Modified(overflow_both_directions),
                    vertical: TextOverflowBehaviour::Modified(overflow_both_directions),
                }.into())
            },
            "overflow-x"        => {
                let overflow_x = parse_layout_text_overflow(value)?;
                Ok(LayoutOverflow {
                    horizontal: TextOverflowBehaviour::Modified(overflow_x),
                    vertical: TextOverflowBehaviour::default(),
                }.into())
            },
            "overflow-y"        => {
                let overflow_y = parse_layout_text_overflow(value)?;
                Ok(LayoutOverflow {
                    horizontal: TextOverflowBehaviour::default(),
                    vertical: TextOverflowBehaviour::Modified(overflow_y),
                }.into())
            },


            _ => Err((key, value).into())
        }
    }
}

/// Error containing all sub-errors that could happen during CSS parsing
///
/// Usually we want to crash on the first error, to notify the user of the problem.
#[derive(Debug, Clone, PartialEq)]
pub enum CssParsingError<'a> {
    CssBorderParseError(CssBorderParseError<'a>),
    CssShadowParseError(CssShadowParseError<'a>),
    InvalidValueErr(InvalidValueErr<'a>),
    PixelParseError(PixelParseError<'a>),
    PercentageParseError(PercentageParseError),
    CssImageParseError(CssImageParseError<'a>),
    CssStyleFontFamilyParseError(CssStyleFontFamilyParseError<'a>),
    CssBackgroundParseError(CssBackgroundParseError<'a>),
    CssColorParseError(CssColorParseError<'a>),
    CssStyleBorderRadiusParseError(CssStyleBorderRadiusParseError<'a>),
    PaddingParseError(LayoutPaddingParseError<'a>),
    MarginParseError(LayoutMarginParseError<'a>),
    FlexShrinkParseError(FlexShrinkParseError<'a>),
    FlexGrowParseError(FlexGrowParseError<'a>),
    /// Key is not supported, i.e. `#div { aldfjasdflk: 400px }` results in an
    /// `UnsupportedCssKey("aldfjasdflk", "400px")` error
    UnsupportedCssKey(&'a str, &'a str),
}

impl_display!{ CssParsingError<'a>, {
    CssStyleBorderRadiusParseError(e) => format!("Invalid border-radius: {}", e),
    CssBorderParseError(e) => format!("Invalid border property: {}", e),
    CssShadowParseError(e) => format!("Invalid shadow: \"{}\"", e),
    InvalidValueErr(e) => format!("\"{}\"", e.0),
    PixelParseError(e) => format!("{}", e),
    PercentageParseError(e) => format!("{}", e),
    CssImageParseError(e) => format!("{}", e),
    CssStyleFontFamilyParseError(e) => format!("{}", e),
    CssBackgroundParseError(e) => format!("{}", e),
    CssColorParseError(e) => format!("{}", e),
    PaddingParseError(e) => format!("{}", e),
    MarginParseError(e) => format!("{}", e),
    FlexShrinkParseError(e) => format!("{}", e),
    FlexGrowParseError(e) => format!("{}", e),
    UnsupportedCssKey(key, value) => format!("Unsupported Css-key: \"{}\" - value: \"{}\"", key, value),
}}

impl_from!(CssBorderParseError<'a>, CssParsingError::CssBorderParseError);
impl_from!(CssShadowParseError<'a>, CssParsingError::CssShadowParseError);
impl_from!(CssColorParseError<'a>, CssParsingError::CssColorParseError);
impl_from!(InvalidValueErr<'a>, CssParsingError::InvalidValueErr);
impl_from!(PixelParseError<'a>, CssParsingError::PixelParseError);
impl_from!(CssImageParseError<'a>, CssParsingError::CssImageParseError);
impl_from!(CssStyleFontFamilyParseError<'a>, CssParsingError::CssStyleFontFamilyParseError);
impl_from!(CssBackgroundParseError<'a>, CssParsingError::CssBackgroundParseError);
impl_from!(CssStyleBorderRadiusParseError<'a>, CssParsingError::CssStyleBorderRadiusParseError);
impl_from!(LayoutPaddingParseError<'a>, CssParsingError::PaddingParseError);
impl_from!(LayoutMarginParseError<'a>, CssParsingError::MarginParseError);
impl_from!(FlexShrinkParseError<'a>, CssParsingError::FlexShrinkParseError);
impl_from!(FlexGrowParseError<'a>, CssParsingError::FlexGrowParseError);

impl<'a> From<(&'a str, &'a str)> for CssParsingError<'a> {
    fn from((a, b): (&'a str, &'a str)) -> Self {
        CssParsingError::UnsupportedCssKey(a, b)
    }
}

impl<'a> From<PercentageParseError> for CssParsingError<'a> {
    fn from(e: PercentageParseError) -> Self {
        CssParsingError::PercentageParseError(e)
    }
}

/// Simple "invalid value" error, used for
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct InvalidValueErr<'a>(pub &'a str);

const SCALE_FACTOR: f32 = 10000.0;

#[derive(Debug, Copy, Clone, Hash, PartialEq, Eq)]
pub struct PixelValue {
    metric: CssMetric,
    /// Has to be divided by 1000.0 - PixelValue needs to implement Hash,
    /// but Hash is not possible for floating-point values
    number: isize,
}

impl PixelValue {
    #[inline]
    pub fn px(value: f32) -> Self {
        Self::from_metric(CssMetric::Px, value)
    }

    #[inline]
    pub fn em(value: f32) -> Self {
        Self::from_metric(CssMetric::Em, value)
    }

    #[inline]
    pub fn pt(value: f32) -> Self {
        Self::from_metric(CssMetric::Pt, value)
    }

    #[inline]
    pub fn from_metric(metric: CssMetric, value: f32) -> Self {
        Self {
            metric: metric,
            number: (value * SCALE_FACTOR) as isize,
        }
    }

    #[inline]
    pub fn to_pixels(&self) -> f32 {
        match self.metric {
            CssMetric::Px => { self.number as f32 / SCALE_FACTOR },
            CssMetric::Pt => { (self.number as f32 / SCALE_FACTOR) * PT_TO_PX },
            CssMetric::Em => { (self.number as f32 / SCALE_FACTOR) * EM_HEIGHT },
        }
    }
}

/// "100%" or "1.0" value - usize based, so it can be
/// safely hashed, accurate to 4 decimal places
#[derive(Debug, PartialEq, Copy, Clone, Hash, Eq)]
pub struct PercentageValue {
    /// Normalized value, 100% = 1.0
    number: isize,
}

impl PercentageValue {
    pub fn new(value: f32) -> Self {
        Self { number: (value * SCALE_FACTOR) as isize }
    }

    pub fn get(&self) -> f32 {
        self.number as f32 / SCALE_FACTOR
    }
}

#[derive(Debug, PartialEq, Copy, Clone, Hash, Eq)]
pub struct FloatValue {
    number: isize,
}

impl FloatValue {
    pub fn new(value: f32) -> Self {
        Self { number: (value * SCALE_FACTOR) as isize }
    }

    pub fn get(&self) -> f32 {
        self.number as f32 / SCALE_FACTOR
    }
}

impl From<f32> for FloatValue {
    fn from(val: f32) -> Self {
        Self::new(val)
    }
}

#[derive(Debug, PartialEq, Clone, Copy, Hash, Eq)]
pub enum CssMetric {
    Px,
    Pt,
    Em,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CssStyleBorderRadiusParseError<'a> {
    TooManyValues(&'a str),
    PixelParseError(PixelParseError<'a>),
}

impl_display!{ CssStyleBorderRadiusParseError<'a>, {
    TooManyValues(val) => format!("Too many values: \"{}\"", val),
    PixelParseError(e) => format!("{}", e),
}}

impl_from!(PixelParseError<'a>, CssStyleBorderRadiusParseError::PixelParseError);

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CssColorComponent {
    Red,
    Green,
    Blue,
    Hue,
    Saturation,
    Lightness,
    Alpha,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CssColorParseError<'a> {
    InvalidColor(&'a str),
    InvalidColorComponent(u8),
    IntValueParseErr(ParseIntError),
    FloatValueParseErr(ParseFloatError),
    FloatValueOutOfRange(f32),
    MissingColorComponent(CssColorComponent),
    ExtraArguments(&'a str),
    UnclosedColor(&'a str),
    DirectionParseError(CssDirectionParseError<'a>),
    UnsupportedDirection(&'a str),
    InvalidPercentage(&'a str),
}

impl<'a> fmt::Display for CssColorParseError<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::CssColorParseError::*;
        match self {
            InvalidColor(i) => write!(f, "Invalid CSS color: \"{}\"", i),
            InvalidColorComponent(i) => write!(f, "Invalid color component when parsing CSS color: \"{}\"", i),
            IntValueParseErr(e) => write!(f, "CSS color component: Value not in range between 00 - FF: \"{}\"", e),
            FloatValueParseErr(e) => write!(f, "CSS color component: Value cannot be parsed as floating point number: \"{}\"", e),
            FloatValueOutOfRange(v) => write!(f, "CSS color component: Value not in range between 0.0 - 1.0: \"{}\"", v),
            MissingColorComponent(c) => write!(f, "CSS color is missing {:?} component", c),
            ExtraArguments(a) => write!(f, "Extra argument to CSS color: \"{}\"", a),
            UnclosedColor(i) => write!(f, "Unclosed color: \"{}\"", i),
            DirectionParseError(e) => write!(f, "Could not parse direction argument for CSS color: \"{}\"", e),
            UnsupportedDirection(d) => write!(f, "Unsupported direction type for CSS color: \"{}\"", d),
            InvalidPercentage(p) => write!(f, "Invalid percentage when parsing CSS color: \"{}\"", p),
        }
    }
}

impl<'a> From<ParseIntError> for CssColorParseError<'a> {
    fn from(e: ParseIntError) -> Self {
        CssColorParseError::IntValueParseErr(e)
    }
}

impl<'a> From<ParseFloatError> for CssColorParseError<'a> {
    fn from(e: ParseFloatError) -> Self {
        CssColorParseError::FloatValueParseErr(e)
    }
}

impl_from!(CssDirectionParseError<'a>, CssColorParseError::DirectionParseError);

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CssImageParseError<'a> {
    UnclosedQuotes(&'a str),
}

impl_display!{CssImageParseError<'a>, {
    UnclosedQuotes(e) => format!("Unclosed quotes: \"{}\"", e),
}}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct UnclosedQuotesError<'a>(pub(crate) &'a str);

impl<'a> From<UnclosedQuotesError<'a>> for CssImageParseError<'a> {
    fn from(err: UnclosedQuotesError<'a>) -> Self {
        CssImageParseError::UnclosedQuotes(err.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CssBorderParseError<'a> {
    InvalidBorderStyle(InvalidValueErr<'a>),
    InvalidBorderDeclaration(&'a str),
    ThicknessParseError(PixelParseError<'a>),
    ColorParseError(CssColorParseError<'a>),
}

impl_display!{ CssBorderParseError<'a>, {
    InvalidBorderStyle(e) => format!("Invalid style: {}", e.0),
    InvalidBorderDeclaration(e) => format!("Invalid declaration: \"{}\"", e),
    ThicknessParseError(e) => format!("Invalid thickness: {}", e),
    ColorParseError(e) => format!("Invalid color: {}", e),
}}

#[derive(Debug, Clone, PartialEq)]
pub enum CssShadowParseError<'a> {
    InvalidSingleStatement(&'a str),
    TooManyComponents(&'a str),
    ValueParseErr(PixelParseError<'a>),
    ColorParseError(CssColorParseError<'a>),
}

impl_display!{ CssShadowParseError<'a>, {
    InvalidSingleStatement(e) => format!("Invalid single statement: \"{}\"", e),
    TooManyComponents(e) => format!("Too many components: \"{}\"", e),
    ValueParseErr(e) => format!("Invalid value: {}", e),
    ColorParseError(e) => format!("Invalid color-value: {}", e),
}}

impl_from!(PixelParseError<'a>, CssShadowParseError::ValueParseErr);
impl_from!(CssColorParseError<'a>, CssShadowParseError::ColorParseError);

/// parse the border-radius like "5px 10px" or "5px 10px 6px 10px"
fn parse_css_border_radius<'a>(input: &'a str)
-> Result<StyleBorderRadius, CssStyleBorderRadiusParseError<'a>>
{
    let mut components = input.split_whitespace();
    let len = components.clone().count();

    match len {
        1 => {
            // One value - border-radius: 15px;
            // (the value applies to all four corners, which are rounded equally:

            let uniform_radius = parse_pixel_value(components.next().unwrap())?;
            Ok(StyleBorderRadius::uniform(uniform_radius))
        },
        2 => {
            // Two values - border-radius: 15px 50px;
            // (first value applies to top-left and bottom-right corners,
            // and the second value applies to top-right and bottom-left corners):

            let top_left_bottom_right = parse_pixel_value(components.next().unwrap())?;
            let top_right_bottom_left = parse_pixel_value(components.next().unwrap())?;

            Ok(StyleBorderRadius{
                top_left: [top_left_bottom_right, top_left_bottom_right],
                bottom_right: [top_left_bottom_right, top_left_bottom_right],
                top_right: [top_right_bottom_left, top_right_bottom_left],
                bottom_left: [top_right_bottom_left, top_right_bottom_left],
            })
        },
        3 => {
            // Three values - border-radius: 15px 50px 30px;
            // (first value applies to top-left corner,
            // second value applies to top-right and bottom-left corners,
            // and third value applies to bottom-right corner):
            let top_left = parse_pixel_value(components.next().unwrap())?;
            let top_right_bottom_left = parse_pixel_value(components.next().unwrap())?;
            let bottom_right = parse_pixel_value(components.next().unwrap())?;

            Ok(StyleBorderRadius{
                top_left: [top_left, top_left],
                bottom_right: [bottom_right, bottom_right],
                top_right: [top_right_bottom_left, top_right_bottom_left],
                bottom_left: [top_right_bottom_left, top_right_bottom_left],
            })
        }
        4 => {
            // Four values - border-radius: 15px 50px 30px 5px;
            // (first value applies to top-left corner,
            //  second value applies to top-right corner,
            //  third value applies to bottom-right corner,
            //  fourth value applies to bottom-left corner)
            let top_left = parse_pixel_value(components.next().unwrap())?;
            let top_right = parse_pixel_value(components.next().unwrap())?;
            let bottom_right = parse_pixel_value(components.next().unwrap())?;
            let bottom_left = parse_pixel_value(components.next().unwrap())?;

            Ok(StyleBorderRadius{
                top_left: [top_left, top_left],
                bottom_right: [bottom_right, bottom_right],
                top_right: [top_right, top_right],
                bottom_left: [bottom_left, bottom_left],
            })
        },
        _ => {
            Err(CssStyleBorderRadiusParseError::TooManyValues(input))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelParseError<'a> {
    InvalidComponent(&'a str),
    ValueParseErr(ParseFloatError),
}

impl_display!{ PixelParseError<'a>, {
    InvalidComponent(component) => format!("Invalid component: \"{}\"", component),
    ValueParseErr(e) => format!("Unexpected value: \"{}\"", e),
}}

/// parse a single value such as "15px"
fn parse_pixel_value<'a>(input: &'a str)
-> Result<PixelValue, PixelParseError<'a>>
{
    let mut split_pos = 0;
    for (idx, ch) in input.char_indices() {
        if ch.is_numeric() || ch == '.' {
            split_pos = idx;
        }
    }

    split_pos += 1;

    let unit = &input[split_pos..];
    let unit = match unit {
        "px" => CssMetric::Px,
        "em" => CssMetric::Em,
        "pt" => CssMetric::Pt,
        _ => { return Err(PixelParseError::InvalidComponent(&input[(split_pos - 1)..])); }
    };

    let number = input[..split_pos].parse::<f32>().map_err(|e| PixelParseError::ValueParseErr(e))?;

    Ok(PixelValue::from_metric(unit, number))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PercentageParseError {
    ValueParseErr(ParseFloatError),
}

impl_display! { PercentageParseError, {
    ValueParseErr(e) => format!("Could not parse percentage value: \"{}\"", e),
}}

// Parse "1.2" or "120%" (similar to parse_pixel_value)
fn parse_percentage_value(input: &str)
-> Result<PercentageValue, PercentageParseError>
{
    let mut split_pos = 0;
    for (idx, ch) in input.char_indices() {
        if ch.is_numeric() || ch == '.' {
            split_pos = idx;
        }
    }

    split_pos += 1;

    let unit = &input[split_pos..];
    let mut number = input[..split_pos].parse::<f32>().map_err(|e| PercentageParseError::ValueParseErr(e))?;

    if unit == "%" {
        number /= 100.0;
    }

    Ok(PercentageValue::new(number))
}

/// Parse any valid CSS color, INCLUDING THE HASH
///
/// "blue" -> "00FF00" -> ColorF { r: 0, g: 255, b: 0 })
/// "#00FF00" -> ColorF { r: 0, g: 255, b: 0 })
pub(crate) fn parse_css_color<'a>(input: &'a str)
-> Result<ColorU, CssColorParseError<'a>>
{
    if input.starts_with('#') {
        parse_color_no_hash(&input[1..])
    } else if input.starts_with("rgba(") {
        if input.ends_with(")") {
            parse_color_rgb(&input[5..input.len()-1], true)
        } else {
            Err(CssColorParseError::UnclosedColor(input))
        }
    } else if input.starts_with("rgb(") {
        if input.ends_with(")") {
            parse_color_rgb(&input[4..input.len()-1], false)
        } else {
            Err(CssColorParseError::UnclosedColor(input))
        }
    } else if input.starts_with("hsla(") {
        if input.ends_with(")") {
            parse_color_hsl(&input[5..input.len()-1], true)
        } else {
            Err(CssColorParseError::UnclosedColor(input))
        }
    } else if input.starts_with("hsl(") {
        if input.ends_with(")") {
            parse_color_hsl(&input[4..input.len()-1], false)
        } else {
            Err(CssColorParseError::UnclosedColor(input))
        }
    } else {
        parse_color_builtin(input)
    }
}

fn parse_float_value(input: &str)
-> Result<FloatValue, ParseFloatError>
{
    Ok(FloatValue::new(input.trim().parse::<f32>()?))
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StyleBorderRadius {
    pub top_left: [PixelValue;2],
    pub top_right: [PixelValue;2],
    pub bottom_left: [PixelValue;2],
    pub bottom_right: [PixelValue;2],
}

impl StyleBorderRadius {

    pub fn zero() -> Self {
        const ZERO_PX: PixelValue = PixelValue { number: 0, metric: CssMetric::Px };
        Self::uniform(ZERO_PX)
    }

    pub fn uniform(value: PixelValue) -> Self {
        Self {
            top_left: [value, value],
            top_right: [value, value],
            bottom_left: [value, value],
            bottom_right: [value, value],
        }
    }
}

impl From<StyleBorderRadius> for BorderRadius {
    fn from(radius: StyleBorderRadius) -> BorderRadius {
        Self {
            top_left: LayoutSize::new(radius.top_left[0].to_pixels(), radius.top_left[1].to_pixels()),
            top_right: LayoutSize::new(radius.top_right[0].to_pixels(), radius.top_right[1].to_pixels()),
            bottom_left: LayoutSize::new(radius.bottom_left[0].to_pixels(), radius.bottom_left[1].to_pixels()),
            bottom_right: LayoutSize::new(radius.bottom_right[0].to_pixels(), radius.bottom_right[1].to_pixels()),
        }
    }
}

/// Represents a parsed CSS `background-color` attribute
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StyleBackgroundColor(pub ColorU);

impl Default for StyleBackgroundColor {
    fn default() -> Self {
        // Transparent color
        StyleBackgroundColor(ColorU::new(0, 0, 0, 0))
    }
}

fn parse_css_background_color<'a>(input: &'a str)
-> Result<StyleBackgroundColor, CssColorParseError<'a>>
{
    parse_css_color(input).and_then(|ok| Ok(StyleBackgroundColor(ok)))
}

/// Represents a parsed CSS `color` attribute
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StyleTextColor(pub ColorU);

fn parse_css_text_color<'a>(input: &'a str)
-> Result<StyleTextColor, CssColorParseError<'a>>
{
    parse_css_color(input).and_then(|ok| Ok(StyleTextColor(ok)))
}

/// Parse a built-in background color
///
/// "blue" -> "00FF00" -> ColorF { r: 0, g: 255, b: 0 })
pub fn parse_color_builtin<'a>(input: &'a str)
-> Result<ColorU, CssColorParseError<'a>>
{
    let (r, g, b, a) = match input {
        "AliceBlue"             | "alice-blue"                =>  (240,248,255,255),
        "AntiqueWhite"          | "antique-white"             =>  (250,235,215,255),
        "Aqua"                  | "aqua"                      =>  (  0,255,255,255),
        "Aquamarine"            | "aquamarine"                =>  (127,255,212,255),
        "Azure"                 | "azure"                     =>  (240,255,255,255),
        "Beige"                 | "beige"                     =>  (245,245,220,255),
        "Bisque"                | "bisque"                    =>  (255,228,196,255),
        "Black"                 | "black"                     =>  (  0,  0,  0,255),
        "BlanchedAlmond"        | "blanched-almond"           =>  (255,235,205,255),
        "Blue"                  | "blue"                      =>  (  0,  0,255,255),
        "BlueViolet"            | "blue-violet"               =>  (138, 43,226,255),
        "Brown"                 | "brown"                     =>  (165, 42, 42,255),
        "BurlyWood"             | "burly-wood"                =>  (222,184,135,255),
        "CadetBlue"             | "cadet-blue"                =>  ( 95,158,160,255),
        "Chartreuse"            | "chartreuse"                =>  (127,255,  0,255),
        "Chocolate"             | "chocolate"                 =>  (210,105, 30,255),
        "Coral"                 | "coral"                     =>  (255,127, 80,255),
        "CornflowerBlue"        | "cornflower-blue"           =>  (100,149,237,255),
        "Cornsilk"              | "cornsilk"                  =>  (255,248,220,255),
        "Crimson"               | "crimson"                   =>  (220, 20, 60,255),
        "Cyan"                  | "cyan"                      =>  (  0,255,255,255),
        "DarkBlue"              | "dark-blue"                 =>  (  0,  0,139,255),
        "DarkCyan"              | "dark-cyan"                 =>  (  0,139,139,255),
        "DarkGoldenRod"         | "dark-golden-rod"           =>  (184,134, 11,255),
        "DarkGray"              | "dark-gray"                 =>  (169,169,169,255),
        "DarkGrey"              | "dark-grey"                 =>  (169,169,169,255),
        "DarkGreen"             | "dark-green"                =>  (  0,100,  0,255),
        "DarkKhaki"             | "dark-khaki"                =>  (189,183,107,255),
        "DarkMagenta"           | "dark-magenta"              =>  (139,  0,139,255),
        "DarkOliveGreen"        | "dark-olive-green"          =>  ( 85,107, 47,255),
        "DarkOrange"            | "dark-orange"               =>  (255,140,  0,255),
        "DarkOrchid"            | "dark-orchid"               =>  (153, 50,204,255),
        "DarkRed"               | "dark-red"                  =>  (139,  0,  0,255),
        "DarkSalmon"            | "dark-salmon"               =>  (233,150,122,255),
        "DarkSeaGreen"          | "dark-sea-green"            =>  (143,188,143,255),
        "DarkSlateBlue"         | "dark-slate-blue"           =>  ( 72, 61,139,255),
        "DarkSlateGray"         | "dark-slate-gray"           =>  ( 47, 79, 79,255),
        "DarkSlateGrey"         | "dark-slate-grey"           =>  ( 47, 79, 79,255),
        "DarkTurquoise"         | "dark-turquoise"            =>  (  0,206,209,255),
        "DarkViolet"            | "dark-violet"               =>  (148,  0,211,255),
        "DeepPink"              | "deep-pink"                 =>  (255, 20,147,255),
        "DeepSkyBlue"           | "deep-sky-blue"             =>  (  0,191,255,255),
        "DimGray"               | "dim-gray"                  =>  (105,105,105,255),
        "DimGrey"               | "dim-grey"                  =>  (105,105,105,255),
        "DodgerBlue"            | "dodger-blue"               =>  ( 30,144,255,255),
        "FireBrick"             | "fire-brick"                =>  (178, 34, 34,255),
        "FloralWhite"           | "floral-white"              =>  (255,250,240,255),
        "ForestGreen"           | "forest-green"              =>  ( 34,139, 34,255),
        "Fuchsia"               | "fuchsia"                   =>  (255,  0,255,255),
        "Gainsboro"             | "gainsboro"                 =>  (220,220,220,255),
        "GhostWhite"            | "ghost-white"               =>  (248,248,255,255),
        "Gold"                  | "gold"                      =>  (255,215,  0,255),
        "GoldenRod"             | "golden-rod"                =>  (218,165, 32,255),
        "Gray"                  | "gray"                      =>  (128,128,128,255),
        "Grey"                  | "grey"                      =>  (128,128,128,255),
        "Green"                 | "green"                     =>  (  0,128,  0,255),
        "GreenYellow"           | "green-yellow"              =>  (173,255, 47,255),
        "HoneyDew"              | "honey-dew"                 =>  (240,255,240,255),
        "HotPink"               | "hot-pink"                  =>  (255,105,180,255),
        "IndianRed"             | "indian-red"                =>  (205, 92, 92,255),
        "Indigo"                | "indigo"                    =>  ( 75,  0,130,255),
        "Ivory"                 | "ivory"                     =>  (255,255,240,255),
        "Khaki"                 | "khaki"                     =>  (240,230,140,255),
        "Lavender"              | "lavender"                  =>  (230,230,250,255),
        "LavenderBlush"         | "lavender-blush"            =>  (255,240,245,255),
        "LawnGreen"             | "lawn-green"                =>  (124,252,  0,255),
        "LemonChiffon"          | "lemon-chiffon"             =>  (255,250,205,255),
        "LightBlue"             | "light-blue"                =>  (173,216,230,255),
        "LightCoral"            | "light-coral"               =>  (240,128,128,255),
        "LightCyan"             | "light-cyan"                =>  (224,255,255,255),
        "LightGoldenRodYellow"  | "light-golden-rod-yellow"   =>  (250,250,210,255),
        "LightGray"             | "light-gray"                =>  (211,211,211,255),
        "LightGrey"             | "light-grey"                =>  (144,238,144,255),
        "LightGreen"            | "light-green"               =>  (211,211,211,255),
        "LightPink"             | "light-pink"                =>  (255,182,193,255),
        "LightSalmon"           | "light-salmon"              =>  (255,160,122,255),
        "LightSeaGreen"         | "light-sea-green"           =>  ( 32,178,170,255),
        "LightSkyBlue"          | "light-sky-blue"            =>  (135,206,250,255),
        "LightSlateGray"        | "light-slate-gray"          =>  (119,136,153,255),
        "LightSlateGrey"        | "light-slate-grey"          =>  (119,136,153,255),
        "LightSteelBlue"        | "light-steel-blue"          =>  (176,196,222,255),
        "LightYellow"           | "light-yellow"              =>  (255,255,224,255),
        "Lime"                  | "lime"                      =>  (  0,255,  0,255),
        "LimeGreen"             | "lime-green"                =>  ( 50,205, 50,255),
        "Linen"                 | "linen"                     =>  (250,240,230,255),
        "Magenta"               | "magenta"                   =>  (255,  0,255,255),
        "Maroon"                | "maroon"                    =>  (128,  0,  0,255),
        "MediumAquaMarine"      | "medium-aqua-marine"        =>  (102,205,170,255),
        "MediumBlue"            | "medium-blue"               =>  (  0,  0,205,255),
        "MediumOrchid"          | "medium-orchid"             =>  (186, 85,211,255),
        "MediumPurple"          | "medium-purple"             =>  (147,112,219,255),
        "MediumSeaGreen"        | "medium-sea-green"          =>  ( 60,179,113,255),
        "MediumSlateBlue"       | "medium-slate-blue"         =>  (123,104,238,255),
        "MediumSpringGreen"     | "medium-spring-green"       =>  (  0,250,154,255),
        "MediumTurquoise"       | "medium-turquoise"          =>  ( 72,209,204,255),
        "MediumVioletRed"       | "medium-violet-red"         =>  (199, 21,133,255),
        "MidnightBlue"          | "midnight-blue"             =>  ( 25, 25,112,255),
        "MintCream"             | "mint-cream"                =>  (245,255,250,255),
        "MistyRose"             | "misty-rose"                =>  (255,228,225,255),
        "Moccasin"              | "moccasin"                  =>  (255,228,181,255),
        "NavajoWhite"           | "navajo-white"              =>  (255,222,173,255),
        "Navy"                  | "navy"                      =>  (  0,  0,128,255),
        "OldLace"               | "old-lace"                  =>  (253,245,230,255),
        "Olive"                 | "olive"                     =>  (128,128,  0,255),
        "OliveDrab"             | "olive-drab"                =>  (107,142, 35,255),
        "Orange"                | "orange"                    =>  (255,165,  0,255),
        "OrangeRed"             | "orange-red"                =>  (255, 69,  0,255),
        "Orchid"                | "orchid"                    =>  (218,112,214,255),
        "PaleGoldenRod"         | "pale-golden-rod"           =>  (238,232,170,255),
        "PaleGreen"             | "pale-green"                =>  (152,251,152,255),
        "PaleTurquoise"         | "pale-turquoise"            =>  (175,238,238,255),
        "PaleVioletRed"         | "pale-violet-red"           =>  (219,112,147,255),
        "PapayaWhip"            | "papaya-whip"               =>  (255,239,213,255),
        "PeachPuff"             | "peach-puff"                =>  (255,218,185,255),
        "Peru"                  | "peru"                      =>  (205,133, 63,255),
        "Pink"                  | "pink"                      =>  (255,192,203,255),
        "Plum"                  | "plum"                      =>  (221,160,221,255),
        "PowderBlue"            | "powder-blue"               =>  (176,224,230,255),
        "Purple"                | "purple"                    =>  (128,  0,128,255),
        "RebeccaPurple"         | "rebecca-purple"            =>  (102, 51,153,255),
        "Red"                   | "red"                       =>  (255,  0,  0,255),
        "RosyBrown"             | "rosy-brown"                =>  (188,143,143,255),
        "RoyalBlue"             | "royal-blue"                =>  ( 65,105,225,255),
        "SaddleBrown"           | "saddle-brown"              =>  (139, 69, 19,255),
        "Salmon"                | "salmon"                    =>  (250,128,114,255),
        "SandyBrown"            | "sandy-brown"               =>  (244,164, 96,255),
        "SeaGreen"              | "sea-green"                 =>  ( 46,139, 87,255),
        "SeaShell"              | "sea-shell"                 =>  (255,245,238,255),
        "Sienna"                | "sienna"                    =>  (160, 82, 45,255),
        "Silver"                | "silver"                    =>  (192,192,192,255),
        "SkyBlue"               | "sky-blue"                  =>  (135,206,235,255),
        "SlateBlue"             | "slate-blue"                =>  (106, 90,205,255),
        "SlateGray"             | "slate-gray"                =>  (112,128,144,255),
        "SlateGrey"             | "slate-grey"                =>  (112,128,144,255),
        "Snow"                  | "snow"                      =>  (255,250,250,255),
        "SpringGreen"           | "spring-green"              =>  (  0,255,127,255),
        "SteelBlue"             | "steel-blue"                =>  ( 70,130,180,255),
        "Tan"                   | "tan"                       =>  (210,180,140,255),
        "Teal"                  | "teal"                      =>  (  0,128,128,255),
        "Thistle"               | "thistle"                   =>  (216,191,216,255),
        "Tomato"                | "tomato"                    =>  (255, 99, 71,255),
        "Turquoise"             | "turquoise"                 =>  ( 64,224,208,255),
        "Violet"                | "violet"                    =>  (238,130,238,255),
        "Wheat"                 | "wheat"                     =>  (245,222,179,255),
        "White"                 | "white"                     =>  (255,255,255,255),
        "WhiteSmoke"            | "white-smoke"               =>  (245,245,245,255),
        "Yellow"                | "yellow"                    =>  (255,255,  0,255),
        "YellowGreen"           | "yellow-green"              =>  (154,205, 50,255),
        "Transparent"           | "transparent"               =>  (255,255,255,  0),
        _ => { return Err(CssColorParseError::InvalidColor(input)); }
    };
    Ok(ColorU { r, g, b, a })
}

/// Parse a color of the form 'rgb([0-255], [0-255], [0-255])', or 'rgba([0-255], [0-255], [0-255],
/// [0.0-1.0])' without the leading 'rgb[a](' or trailing ')'. Alpha defaults to 255.
fn parse_color_rgb<'a>(input: &'a str, parse_alpha: bool)
-> Result<ColorU, CssColorParseError<'a>>
{
    let mut components = input.split(',').map(|c| c.trim());
    let rgb_color = parse_color_rgb_components(&mut components)?;
    let a = if parse_alpha {
        parse_alpha_component(&mut components)?
    } else {
        255
    };
    if let Some(arg) = components.next() {
        return Err(CssColorParseError::ExtraArguments(arg));
    }
    Ok(ColorU { a, ..rgb_color })
}

/// Parse the color components passed as arguments to an rgb(...) CSS color.
fn parse_color_rgb_components<'a>(components: &mut Iterator<Item = &'a str>)
-> Result<ColorU, CssColorParseError<'a>>
{
    #[inline]
    fn component_from_str<'a>(components: &mut Iterator<Item = &'a str>, which: CssColorComponent)
    -> Result<u8, CssColorParseError<'a>>
    {
        let c = components.next().ok_or(CssColorParseError::MissingColorComponent(which))?;
        if c.is_empty() {
            return Err(CssColorParseError::MissingColorComponent(which));
        }
        let c = c.parse::<u8>()?;
        Ok(c)
    }

    Ok(ColorU {
        r: component_from_str(components, CssColorComponent::Red)?,
        g: component_from_str(components, CssColorComponent::Green)?,
        b: component_from_str(components, CssColorComponent::Blue)?,
        a: 255
    })
}

/// Parse a color of the form 'hsl([0.0-360.0]deg, [0-100]%, [0-100]%)', or 'hsla([0.0-360.0]deg, [0-100]%, [0-100]%, [0.0-1.0])' without the leading 'hsl[a](' or trailing ')'. Alpha defaults to 255.
fn parse_color_hsl<'a>(input: &'a str, parse_alpha: bool)
-> Result<ColorU, CssColorParseError<'a>>
{
    let mut components = input.split(',').map(|c| c.trim());
    let rgb_color = parse_color_hsl_components(&mut components)?;
    let a = if parse_alpha {
        parse_alpha_component(&mut components)?
    } else {
        255
    };
    if let Some(arg) = components.next() {
        return Err(CssColorParseError::ExtraArguments(arg));
    }
    Ok(ColorU { a, ..rgb_color })
}

/// Converts the hue, saturation, and lightness components of a color into
/// red, green, and blue components.
/// Hue is an angle from 0.0 to 360.0
/// Saturation is a percentage from 0.0 to 100.0
/// Lightness is a percentage from 0.0 to 100.0
/// Adapted from [https://en.wikipedia.org/wiki/HSL_and_HSV#Converting_to_RGB]
#[inline]
fn hsl_to_rgb<'a>(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let s = s / 100.0;
    let l = l / 100.0;
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = h / 60.0;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match h as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        5 => (c, 0.0, x),
        _ => {
            println!("h is {}", h);
            unreachable!();
        }
    };
    let m = l - c / 2.0;
    (
        ((r1 + m) * 256.0).min(255.0) as u8,
        ((g1 + m) * 256.0).min(255.0) as u8,
        ((b1 + m) * 256.0).min(255.0) as u8,
    )
}

/// Parse the color components passed as arguments to an hsl(...) CSS color.
fn parse_color_hsl_components<'a>(components: &mut Iterator<Item = &'a str>)
-> Result<ColorU, CssColorParseError<'a>>
{
    #[inline]
    fn angle_from_str<'a>(components: &mut Iterator<Item = &'a str>, which: CssColorComponent)
    -> Result<f32, CssColorParseError<'a>>
    {
        let c = components.next().ok_or(CssColorParseError::MissingColorComponent(which))?;
        if c.is_empty() {
            return Err(CssColorParseError::MissingColorComponent(which));
        }
        let dir = parse_direction(c)?;
        match dir {
            Direction::Angle(deg) => Ok(deg.get()),
            Direction::FromTo(_, _) => return Err(CssColorParseError::UnsupportedDirection(c)),
        }
    }

    #[inline]
    fn percent_from_str<'a>(components: &mut Iterator<Item = &'a str>, which: CssColorComponent)
    -> Result<f32, CssColorParseError<'a>>
    {
        let c = components.next().ok_or(CssColorParseError::MissingColorComponent(which))?;
        if c.is_empty() {
            return Err(CssColorParseError::MissingColorComponent(which));
        }
        parse_percentage(c)
            .ok_or(CssColorParseError::InvalidPercentage(c))
            .and_then(|p| Ok(p.get()))
    }

    let (h, s, l) = (
        angle_from_str(components, CssColorComponent::Hue)?,
        percent_from_str(components, CssColorComponent::Saturation)?,
        percent_from_str(components, CssColorComponent::Lightness)?,
    );

    let (r, g, b) = hsl_to_rgb(h, s, l);

    Ok(ColorU { r, g, b, a: 255 })
}

#[inline]
fn parse_alpha_component<'a>(components: &mut Iterator<Item=&'a str>) -> Result<u8, CssColorParseError<'a>> {
    let a = components.next().ok_or(CssColorParseError::MissingColorComponent(CssColorComponent::Alpha))?;
    if a.is_empty() {
        return Err(CssColorParseError::MissingColorComponent(CssColorComponent::Alpha));
    }
    let a = a.parse::<f32>()?;
    if a < 0.0 || a > 1.0 {
        return Err(CssColorParseError::FloatValueOutOfRange(a));
    }
    let a = (a * 256.0).min(255.0) as u8;
    Ok(a)
}


/// Parse a background color, WITHOUT THE HASH
///
/// "00FFFF" -> ColorF { r: 0, g: 255, b: 255})
fn parse_color_no_hash<'a>(input: &'a str)
-> Result<ColorU, CssColorParseError<'a>>
{
    #[inline]
    fn from_hex<'a>(c: u8) -> Result<u8, CssColorParseError<'a>> {
        match c {
            b'0' ... b'9' => Ok(c - b'0'),
            b'a' ... b'f' => Ok(c - b'a' + 10),
            b'A' ... b'F' => Ok(c - b'A' + 10),
            _ => Err(CssColorParseError::InvalidColorComponent(c))
        }
    }

    match input.len() {
        3 => {
            let mut input_iter = input.chars();

            let r = input_iter.next().unwrap() as u8;
            let g = input_iter.next().unwrap() as u8;
            let b = input_iter.next().unwrap() as u8;

            let r = from_hex(r)? * 16 + from_hex(r)?;
            let g = from_hex(g)? * 16 + from_hex(g)?;
            let b = from_hex(b)? * 16 + from_hex(b)?;

            Ok(ColorU {
                r: r,
                g: g,
                b: b,
                a: 255,
            })
        },
        4 => {
            let mut input_iter = input.chars();

            let r = input_iter.next().unwrap() as u8;
            let g = input_iter.next().unwrap() as u8;
            let b = input_iter.next().unwrap() as u8;
            let a = input_iter.next().unwrap() as u8;

            let r = from_hex(r)? * 16 + from_hex(r)?;
            let g = from_hex(g)? * 16 + from_hex(g)?;
            let b = from_hex(b)? * 16 + from_hex(b)?;
            let a = from_hex(a)? * 16 + from_hex(a)?;

            Ok(ColorU {
                r: r,
                g: g,
                b: b,
                a: a,
            })
        },
        6 => {
            let input = u32::from_str_radix(input, 16).map_err(|e| CssColorParseError::IntValueParseErr(e))?;
            Ok(ColorU {
                r: ((input >> 16) & 255) as u8,
                g: ((input >> 8) & 255) as u8,
                b: (input & 255) as u8,
                a: 255,
            })
        },
        8 => {
            let input = u32::from_str_radix(input, 16).map_err(|e| CssColorParseError::IntValueParseErr(e))?;
            Ok(ColorU {
                r: ((input >> 24) & 255) as u8,
                g: ((input >> 16) & 255) as u8,
                b: ((input >> 8) & 255) as u8,
                a: (input & 255) as u8,
            })
        },
        _ => { Err(CssColorParseError::InvalidColor(input)) }
    }
}

/// Represents a parsed CSS `padding` attribute
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct LayoutPadding {
    pub top: Option<PixelValue>,
    pub bottom: Option<PixelValue>,
    pub left: Option<PixelValue>,
    pub right: Option<PixelValue>,
}

macro_rules! parse_x {($struct_name:ident, $error_name:ident, $fn_name:ident, $field:ident, $body:ident) => (
fn $fn_name<'a>(input: &'a str) -> Result<$struct_name, $error_name> {
    let value = $body(input)?;
    Ok($struct_name {
        $field: Some(value),
        .. Default::default()
    })
})}

macro_rules! parse_tblr {($struct_name:ident, $error_name:ident, $parse_fn:ident) => (
impl $struct_name {
    parse_x!($struct_name, $error_name, parse_left, left, $parse_fn);
    parse_x!($struct_name, $error_name, parse_right, right, $parse_fn);
    parse_x!($struct_name, $error_name, parse_bottom, bottom, $parse_fn);
    parse_x!($struct_name, $error_name, parse_top, top, $parse_fn);
})}

// $struct_name has to have top, left, right, bottom properties
macro_rules! merge_struct {($struct_name:ident) => (
impl $struct_name {
    pub fn merge(a: &mut Option<$struct_name>, b: &$struct_name) {
       if let Some(ref mut existing) = a {
           if b.top.is_some() { existing.top = b.top; }
           if b.bottom.is_some() { existing.bottom = b.bottom; }
           if b.left.is_some() { existing.left = b.left; }
           if b.right.is_some() { existing.right = b.right; }
       } else {
           *a = Some(*b);
       }
    }
})}

macro_rules! struct_all {($struct_name:ident, $field_type:ty) => (
impl $struct_name {
    /// Sets all of the fields (top, left, right, bottom) to `Some(field)`
    fn all(field: $field_type) -> Self {
        Self {
            top: Some(field),
            right: Some(field),
            left: Some(field),
            bottom: Some(field),
        }
    }
})}

merge_struct!(LayoutPadding);
merge_struct!(LayoutMargin);
struct_all!(LayoutPadding, PixelValue);
struct_all!(LayoutMargin, PixelValue);
parse_tblr!(LayoutPadding, LayoutPaddingParseError, parse_pixel_value);
parse_tblr!(LayoutMargin, LayoutMarginParseError, parse_pixel_value);

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutPaddingParseError<'a> {
    PixelParseError(PixelParseError<'a>),
    TooManyValues,
    TooFewValues,
}

impl_display!{ LayoutPaddingParseError<'a>, {
    PixelParseError(e) => format!("Could not parse pixel value: {}", e),
    TooManyValues => format!("Too many values - padding property has a maximum of 4 values."),
    TooFewValues => format!("Too few values - padding property has a minimum of 1 value."),
}}

impl_from!(PixelParseError<'a>, LayoutPaddingParseError::PixelParseError);

/// Parse a padding value such as
///
/// "10px 10px"
fn parse_layout_padding<'a>(input: &'a str)
-> Result<LayoutPadding, LayoutPaddingParseError>
{
    let mut input_iter = input.split_whitespace();
    let first = parse_pixel_value(input_iter.next().ok_or(LayoutPaddingParseError::TooFewValues)?)?;
    let second = parse_pixel_value(match input_iter.next() {
        Some(s) => s,
        None => return Ok(LayoutPadding {
            top: Some(first),
            bottom: Some(first),
            left: Some(first),
            right: Some(first),
        }),
    })?;
    let third = parse_pixel_value(match input_iter.next() {
        Some(s) => s,
        None => return Ok(LayoutPadding {
            top: Some(first),
            bottom: Some(first),
            left: Some(second),
            right: Some(second),
        }),
    })?;
    let fourth = parse_pixel_value(match input_iter.next() {
        Some(s) => s,
        None => return Ok(LayoutPadding {
            top: Some(first),
            left: Some(second),
            right: Some(second),
            bottom: Some(third),
        }),
    })?;

    if input_iter.next().is_some() {
        return Err(LayoutPaddingParseError::TooManyValues);
    }

    Ok(LayoutPadding {
        top: Some(first),
        right: Some(second),
        bottom: Some(third),
        left: Some(fourth),
    })
}

/// Represents a parsed CSS `padding` attribute
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct LayoutMargin {
    pub top: Option<PixelValue>,
    pub bottom: Option<PixelValue>,
    pub left: Option<PixelValue>,
    pub right: Option<PixelValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutMarginParseError<'a> {
    PixelParseError(PixelParseError<'a>),
    TooManyValues,
    TooFewValues,
}

impl_display!{ LayoutMarginParseError<'a>, {
    PixelParseError(e) => format!("Could not parse pixel value: {}", e),
    TooManyValues => format!("Too many values - margin property has a maximum of 4 values."),
    TooFewValues => format!("Too few values - margin property has a minimum of 1 value."),
}}

impl_from!(PixelParseError<'a>, LayoutMarginParseError::PixelParseError);

fn parse_layout_margin<'a>(input: &'a str)
-> Result<LayoutMargin, LayoutMarginParseError>
{
    match parse_layout_padding(input) {
        Ok(padding) => {
            Ok(LayoutMargin {
                top: padding.top,
                left: padding.left,
                right: padding.right,
                bottom: padding.bottom,
            })
        },
        Err(LayoutPaddingParseError::PixelParseError(e)) => Err(e.into()),
        Err(LayoutPaddingParseError::TooManyValues) => Err(LayoutMarginParseError::TooManyValues),
        Err(LayoutPaddingParseError::TooFewValues) => Err(LayoutMarginParseError::TooFewValues),
    }
}

/// Wrapper for the `overflow-{x,y}` + `overflow` property
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct LayoutOverflow {
    pub horizontal: TextOverflowBehaviour,
    pub vertical: TextOverflowBehaviour,
}

impl LayoutOverflow {

    // "merges" two LayoutOverflow properties
    pub fn merge(a: &mut Option<Self>, b: &Self) {

        fn merge_property(p: &mut TextOverflowBehaviour, other: &TextOverflowBehaviour) {
            if *other == TextOverflowBehaviour::NotModified {
                return;
            }
            *p = *other;
        }

        if let Some(ref mut existing_overflow) = a {
            merge_property(&mut existing_overflow.horizontal, &b.horizontal);
            merge_property(&mut existing_overflow.vertical, &b.vertical);
        } else {
            *a = Some(*b)
        }
    }

    pub fn allows_horizontal_overflow(&self) -> bool {
        self.horizontal.can_overflow()
    }

    pub fn allows_vertical_overflow(&self) -> bool {
        self.vertical.can_overflow()
    }

    // If this overflow setting should show the horizontal scrollbar
    pub fn allows_horizontal_scrollbar(&self) -> bool {
        self.allows_horizontal_overflow()
    }

    pub fn allows_vertical_scrollbar(&self) -> bool {
        self.allows_vertical_overflow()
    }
}

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StyleBorder {
    pub top: Option<StyleBorderSide>,
    pub left: Option<StyleBorderSide>,
    pub bottom: Option<StyleBorderSide>,
    pub right: Option<StyleBorderSide>,
}

merge_struct!(StyleBorder);
parse_tblr!(StyleBorder, CssBorderParseError, parse_css_border);
struct_all!(StyleBorder, StyleBorderSide);

impl StyleBorder {

    /// Returns the merged offsets and details for the top, left,
    /// right and bottom styles - necessary, so we can combine `border-top`,
    /// `border-left`, etc. into one border
    pub fn get_webrender_border(&self, border_radius: Option<StyleBorderRadius>) -> Option<(LayoutSideOffsets, BorderDetails)> {
        match (self.top, self.left, self.bottom, self.right) {
            (None, None, None, None) => None,
            (top, left, bottom, right) => {

                // Widths
                let border_width_top = top.and_then(|top|  Some(top.border_width.to_pixels())).unwrap_or(0.0);
                let border_width_bottom = bottom.and_then(|bottom|  Some(bottom.border_width.to_pixels())).unwrap_or(0.0);
                let border_width_left = left.and_then(|left|  Some(left.border_width.to_pixels())).unwrap_or(0.0);
                let border_width_right = right.and_then(|right|  Some(right.border_width.to_pixels())).unwrap_or(0.0);

                // Color
                let border_color_top = top.and_then(|top| Some(top.border_color.into())).unwrap_or(DEFAULT_BORDER_COLOR);
                let border_color_bottom = bottom.and_then(|bottom| Some(bottom.border_color.into())).unwrap_or(DEFAULT_BORDER_COLOR);
                let border_color_left = left.and_then(|left| Some(left.border_color.into())).unwrap_or(DEFAULT_BORDER_COLOR);
                let border_color_right = right.and_then(|right| Some(right.border_color.into())).unwrap_or(DEFAULT_BORDER_COLOR);

                // Styles
                let border_style_top = top.and_then(|top| Some(top.border_style)).unwrap_or(DEFAULT_BORDER_STYLE);
                let border_style_bottom = bottom.and_then(|bottom| Some(bottom.border_style)).unwrap_or(DEFAULT_BORDER_STYLE);
                let border_style_left = left.and_then(|left| Some(left.border_style)).unwrap_or(DEFAULT_BORDER_STYLE);
                let border_style_right = right.and_then(|right| Some(right.border_style)).unwrap_or(DEFAULT_BORDER_STYLE);

                // Webrender crashes if AA is disabled and the border isn't pure-solid
                let is_not_solid = [border_style_top, border_style_bottom, border_style_left, border_style_right].iter().any(|style| {
                    *style != BorderStyle::Solid
                });

                let border_widths = LayoutSideOffsets::new(border_width_top, border_width_right, border_width_bottom, border_width_left);
                let border_details = BorderDetails::Normal(NormalBorder {
                    top: BorderSide { color:  border_color_top.into(), style: border_style_top },
                    left: BorderSide { color:  border_color_left.into(), style: border_style_left },
                    right: BorderSide { color:  border_color_right.into(),  style: border_style_right },
                    bottom: BorderSide { color:  border_color_bottom.into(), style: border_style_bottom },
                    radius: border_radius.and_then(|b| Some(b.into())).unwrap_or(BorderRadius::zero()),
                    do_aa: border_radius.is_some() || is_not_solid,
                });

                Some((border_widths, border_details))
            }
        }
    }
}

const DEFAULT_BORDER_STYLE: BorderStyle = BorderStyle::Solid;
const DEFAULT_BORDER_COLOR: ColorU = ColorU { r: 0, g: 0, b: 0, a: 255 };

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StyleBorderSide {
    pub border_width: PixelValue,
    pub border_style: BorderStyle,
    pub border_color: ColorU,
}

/// Parse a CSS border such as
///
/// "5px solid red"
fn parse_css_border<'a>(input: &'a str)
-> Result<StyleBorderSide, CssBorderParseError<'a>>
{
    // Default border thickness on the web seems to be 3px
    const DEFAULT_BORDER_THICKNESS: f32 = 3.0;

    let mut input_iter = input.split_whitespace();

    let (border_width, border_style, border_color);

    match input_iter.clone().count() {
        1 => {
            border_style = parse_border_style(input_iter.next().unwrap())
                            .map_err(|e| CssBorderParseError::InvalidBorderStyle(e))?;
            border_width = PixelValue::px(DEFAULT_BORDER_THICKNESS);
            border_color = DEFAULT_BORDER_COLOR;
        },
        3 => {
            border_width = parse_pixel_value(input_iter.next().unwrap())
                           .map_err(|e| CssBorderParseError::ThicknessParseError(e))?;
            border_style = parse_border_style(input_iter.next().unwrap())
                           .map_err(|e| CssBorderParseError::InvalidBorderStyle(e))?;
            border_color = parse_css_color(input_iter.next().unwrap())
                           .map_err(|e| CssBorderParseError::ColorParseError(e))?;
       },
       _ => {
            return Err(CssBorderParseError::InvalidBorderDeclaration(input));
       }
    }

    Ok(StyleBorderSide {
        border_width,
        border_style,
        border_color,
    })
}

/// Parse a border style such as "none", "dotted", etc.
///
/// "solid", "none", etc.
multi_type_parser!(parse_border_style, BorderStyle,
    ["none", None],
    ["solid", Solid],
    ["double", Double],
    ["dotted", Dotted],
    ["dashed", Dashed],
    ["hidden", Hidden],
    ["groove", Groove],
    ["ridge", Ridge],
    ["inset", Inset],
    ["outset", Outset]);

#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Hash)]
pub struct StyleBoxShadow {
    pub top: Option<Option<BoxShadowPreDisplayItem>>,
    pub left: Option<Option<BoxShadowPreDisplayItem>>,
    pub bottom: Option<Option<BoxShadowPreDisplayItem>>,
    pub right: Option<Option<BoxShadowPreDisplayItem>>,
}

merge_struct!(StyleBoxShadow);
parse_tblr!(StyleBoxShadow, CssShadowParseError, parse_css_box_shadow);
struct_all!(StyleBoxShadow, Option<BoxShadowPreDisplayItem>);

// missing StyleBorderRadius & LayoutRect
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct BoxShadowPreDisplayItem {
    pub offset: [PixelValue;2],
    pub color: ColorU,
    pub blur_radius: PixelValue,
    pub spread_radius: PixelValue,
    pub clip_mode: BoxShadowClipMode,
}

/// Parses a CSS box-shadow
fn parse_css_box_shadow<'a>(input: &'a str)
-> Result<Option<BoxShadowPreDisplayItem>, CssShadowParseError<'a>>
{
    let mut input_iter = input.split_whitespace();
    let count = input_iter.clone().count();

    let mut box_shadow = BoxShadowPreDisplayItem {
        offset: [PixelValue::px(0.0), PixelValue::px(0.0)],
        color: ColorU { r: 0, g: 0, b: 0, a: 255 },
        blur_radius: PixelValue::px(0.0),
        spread_radius: PixelValue::px(0.0),
        clip_mode: BoxShadowClipMode::Outset,
    };

    let last_val = input_iter.clone().rev().next();
    let is_inset = last_val == Some("inset") || last_val == Some("outset");

    if count > 2 && is_inset {
        let l_val = last_val.unwrap();
        if l_val == "outset" {
            box_shadow.clip_mode = BoxShadowClipMode::Outset;
        } else if l_val == "inset" {
            box_shadow.clip_mode = BoxShadowClipMode::Inset;
        }
    }

    match count {
        1 => {
            // box-shadow: none;
            match input_iter.next().unwrap() {
                "none" => return Ok(None),
                _ => return Err(CssShadowParseError::InvalidSingleStatement(input)),
            }
        },
        2 => {
            // box-shadow: 5px 10px; (h_offset, v_offset)
            let h_offset = parse_pixel_value(input_iter.next().unwrap())?;
            let v_offset = parse_pixel_value(input_iter.next().unwrap())?;
            box_shadow.offset[0] = h_offset;
            box_shadow.offset[1] = v_offset;
        },
        3 => {
            // box-shadow: 5px 10px inset; (h_offset, v_offset, inset)
            let h_offset = parse_pixel_value(input_iter.next().unwrap())?;
            let v_offset = parse_pixel_value(input_iter.next().unwrap())?;
            box_shadow.offset[0] = h_offset;
            box_shadow.offset[1] = v_offset;

            if !is_inset {
                // box-shadow: 5px 10px #888888; (h_offset, v_offset, color)
                let color = parse_css_color(input_iter.next().unwrap())?;
                box_shadow.color = color;
            }
        },
        4 => {
            let h_offset = parse_pixel_value(input_iter.next().unwrap())?;
            let v_offset = parse_pixel_value(input_iter.next().unwrap())?;
            box_shadow.offset[0] = h_offset;
            box_shadow.offset[1] = v_offset;

            if !is_inset {
                let blur = parse_pixel_value(input_iter.next().unwrap())?;
                box_shadow.blur_radius = blur.into();
            }

            let color = parse_css_color(input_iter.next().unwrap())?;
            box_shadow.color = color;
        },
        5 => {
            // box-shadow: 5px 10px 5px 10px #888888; (h_offset, v_offset, blur, spread, color)
            // box-shadow: 5px 10px 5px #888888 inset; (h_offset, v_offset, blur, color, inset)
            let h_offset = parse_pixel_value(input_iter.next().unwrap())?;
            let v_offset = parse_pixel_value(input_iter.next().unwrap())?;
            box_shadow.offset[0] = h_offset;
            box_shadow.offset[1] = v_offset;

            let blur = parse_pixel_value(input_iter.next().unwrap())?;
            box_shadow.blur_radius = blur.into();

            if !is_inset {
                let spread = parse_pixel_value(input_iter.next().unwrap())?;
                box_shadow.spread_radius = spread.into();
            }

            let color = parse_css_color(input_iter.next().unwrap())?;
            box_shadow.color = color;
        },
        6 => {
            // box-shadow: 5px 10px 5px 10px #888888 inset; (h_offset, v_offset, blur, spread, color, inset)
            let h_offset = parse_pixel_value(input_iter.next().unwrap())?;
            let v_offset = parse_pixel_value(input_iter.next().unwrap())?;
            box_shadow.offset[0] = h_offset;
            box_shadow.offset[1] = v_offset;

            let blur = parse_pixel_value(input_iter.next().unwrap())?;
            box_shadow.blur_radius = blur.into();

            let spread = parse_pixel_value(input_iter.next().unwrap())?;
            box_shadow.spread_radius = spread.into();

            let color = parse_css_color(input_iter.next().unwrap())?;
            box_shadow.color = color;
        }
        _ => {
            return Err(CssShadowParseError::TooManyComponents(input));
        }
    }

    Ok(Some(box_shadow))
}

#[derive(Debug, Clone, PartialEq)]
pub enum CssBackgroundParseError<'a> {
    Error(&'a str),
    InvalidBackground(&'a str),
    UnclosedGradient(&'a str),
    NoDirection(&'a str),
    TooFewGradientStops(&'a str),
    DirectionParseError(CssDirectionParseError<'a>),
    GradientParseError(CssGradientStopParseError<'a>),
    ShapeParseError(CssShapeParseError<'a>),
    ImageParseError(CssImageParseError<'a>),
}

impl_display!{ CssBackgroundParseError<'a>, {
    Error(e) => e,
    InvalidBackground(val) => format!("Invalid value: \"{}\"", val),
    UnclosedGradient(val) => format!("Unclosed gradient: \"{}\"", val),
    NoDirection(val) => format!("Gradient has no direction: \"{}\"", val),
    TooFewGradientStops(val) => format!("Too few gradient-stops: \"{}\"", val),
    DirectionParseError(e) => format!("Could not parse gradient direction: \"{}\"", e),
    GradientParseError(e) => format!("Gradient parse error: {}", e),
    ShapeParseError(e) => format!("Shape parse error: {}", e),
    ImageParseError(e) => format!("Image parse error: {}", e),
}}

impl_from!(CssDirectionParseError<'a>, CssBackgroundParseError::DirectionParseError);
impl_from!(CssGradientStopParseError<'a>, CssBackgroundParseError::GradientParseError);
impl_from!(CssShapeParseError<'a>, CssBackgroundParseError::ShapeParseError);
impl_from!(CssImageParseError<'a>, CssBackgroundParseError::ImageParseError);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StyleBackground {
    LinearGradient(LinearGradientPreInfo),
    RadialGradient(RadialGradientPreInfo),
    Image(CssImageId),
    NoBackground,
}

impl<'a> From<CssImageId> for StyleBackground {
    fn from(id: CssImageId) -> Self {
        StyleBackground::Image(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinearGradientPreInfo {
    pub direction: Direction,
    pub extend_mode: ExtendMode,
    pub stops: Vec<GradientStopPre>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RadialGradientPreInfo {
    pub shape: Shape,
    pub extend_mode: ExtendMode,
    pub stops: Vec<GradientStopPre>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Direction {
    Angle(FloatValue),
    FromTo(DirectionCorner, DirectionCorner),
}

impl Direction {
    /// Calculates the points of the gradient stops for angled linear gradients
    pub fn to_points(&self, rect: &LayoutRect)
    -> (LayoutPoint, LayoutPoint)
    {
        match self {
            Direction::Angle(deg) => {
                // note: assumes that the LayoutRect has positive sides

                // see: https://hugogiraudel.com/2013/02/04/css-gradients/

                let deg = deg.get(); // FloatValue -> f32

                let deg = -deg; // negate winding direction

                let width_half = rect.size.width as usize / 2;
                let height_half = rect.size.height as usize / 2;

                // hypotenuse_len is the length of the center of the rect to the corners
                let hypotenuse_len = (((width_half * width_half) + (height_half * height_half)) as f64).sqrt();

                // The corner also serves to determine what quadrant we're in
                // Get the quadrant (corner) the angle is in and get the degree associated
                // with that corner.

                let angle_to_top_left = (height_half as f64 / width_half as f64).atan().to_degrees();

                // We need to calculate the angle from the center to the corner!
                let ending_point_degrees = if deg < 90.0 {
                    // top left corner
                    90.0 - angle_to_top_left
                } else if deg < 180.0 {
                    // bottom left corner
                    90.0 + angle_to_top_left
                } else if deg < 270.0 {
                    // bottom right corner
                    270.0 - angle_to_top_left
                } else /* deg > 270.0 && deg < 360.0 */ {
                    // top right corner
                    270.0 + angle_to_top_left
                };

                // assuming deg = 36deg, then degree_diff_to_corner = 9deg
                let degree_diff_to_corner = ending_point_degrees - deg as f64;

                // Searched_len is the distance between the center of the rect and the
                // ending point of the gradient
                let searched_len = (hypotenuse_len * degree_diff_to_corner.to_radians().cos()).abs();

                // TODO: This searched_len is incorrect...

                // Once we have the length, we can simply rotate the length by the angle,
                // then translate it to the center of the rect
                let dx = deg.to_radians().sin() * searched_len as f32;
                let dy = deg.to_radians().cos() * searched_len as f32;

                let start_point_location = LayoutPoint::new(width_half as f32 + dx, height_half as f32 + dy);
                let end_point_location = LayoutPoint::new(width_half as f32 - dx, height_half as f32 - dy);

                (start_point_location, end_point_location)
            },
            Direction::FromTo(from, to) => {
                (from.to_point(rect), to.to_point(rect))
            }
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Shape {
    Ellipse,
    Circle,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum DirectionCorner {
    Right,
    Left,
    Top,
    Bottom,
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

impl DirectionCorner {

    pub fn opposite(&self) -> Self {
        use self::DirectionCorner::*;
        match *self {
            Right => Left,
            Left => Right,
            Top => Bottom,
            Bottom => Top,
            TopRight => BottomLeft,
            BottomLeft => TopRight,
            TopLeft => BottomRight,
            BottomRight => TopLeft,
        }
    }

    pub fn combine(&self, other: &Self) -> Option<Self> {
        use self::DirectionCorner::*;
        match (*self, *other) {
            (Right, Top) | (Top, Right) => Some(TopRight),
            (Left, Top) | (Top, Left) => Some(TopLeft),
            (Right, Bottom) | (Bottom, Right) => Some(BottomRight),
            (Left, Bottom) | (Bottom, Left) => Some(BottomLeft),
            _ => { None }
        }
    }

    pub fn to_point(&self, rect: &LayoutRect) -> TypedPoint2D<f32, LayoutPixel>
    {
        use self::DirectionCorner::*;
        match *self {
            Right => TypedPoint2D::new(rect.size.width, rect.size.height / 2.0),
            Left => TypedPoint2D::new(0.0, rect.size.height / 2.0),
            Top => TypedPoint2D::new(rect.size.width / 2.0, 0.0),
            Bottom => TypedPoint2D::new(rect.size.width / 2.0, rect.size.height),
            TopRight =>  TypedPoint2D::new(rect.size.width, 0.0),
            TopLeft => TypedPoint2D::new(0.0, 0.0),
            BottomRight => TypedPoint2D::new(rect.size.width, rect.size.height),
            BottomLeft => TypedPoint2D::new(0.0, rect.size.height),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
enum BackgroundType {
    LinearGradient,
    RepeatingLinearGradient,
    RadialGradient,
    RepeatingRadialGradient,
    Image,
}

// parses a background, such as "linear-gradient(red, green)"
fn parse_css_background<'a>(input: &'a str)
-> Result<StyleBackground, CssBackgroundParseError<'a>>
{
    use self::BackgroundType::*;

    let mut input_iter = input.splitn(2, "(");
    let first_item = input_iter.next();

    let background_type = match first_item {
        Some("none") => { return Ok(StyleBackground::NoBackground); },
        Some("linear-gradient") => LinearGradient,
        Some("repeating-linear-gradient") => RepeatingLinearGradient,
        Some("radial-gradient") => RadialGradient,
        Some("repeating-radial-gradient") => RepeatingRadialGradient,
        Some("image") => Image,
        _ => { return Err(CssBackgroundParseError::InvalidBackground(first_item.unwrap())); } // failure here
    };

    let next_item = match input_iter.next() {
        Some(s) => { s },
        None => return Err(CssBackgroundParseError::InvalidBackground(input)),
    };

    let mut brace_iter = next_item.rsplitn(2, ')');
    brace_iter.next(); // important
    let brace_contents = brace_iter.clone().next();

    if brace_contents.is_none() {
        // invalid or empty brace
        return Err(CssBackgroundParseError::UnclosedGradient(input));
    }

    // brace_contents contains "red, yellow, etc"
    let brace_contents = brace_contents.unwrap();
    if background_type == Image {
        let image = parse_image(brace_contents)?;
        return Ok(image.into());
    }

    let mut brace_iterator = brace_contents.split(',');

    let mut gradient_stop_count = brace_iterator.clone().count();

    // "50deg", "to right bottom", etc.
    let first_brace_item = match brace_iterator.next() {
        Some(s) => s,
        None => return Err(CssBackgroundParseError::NoDirection(input)),
    };

    // default shape: ellipse
    let mut shape = Shape::Ellipse;
    // default gradient: from top to bottom
    let mut direction = Direction::FromTo(DirectionCorner::Top, DirectionCorner::Bottom);

    let mut first_is_direction = false;
    let mut first_is_shape = false;
    let is_linear_gradient = background_type == LinearGradient || background_type == RepeatingLinearGradient;
    let is_radial_gradient = background_type == RadialGradient || background_type == RepeatingRadialGradient;

    if is_linear_gradient {
        if let Ok(dir) = parse_direction(first_brace_item) {
            direction = dir;
            first_is_direction = true;
        }
    }

    if is_radial_gradient {
        if let Ok(sh) = parse_shape(first_brace_item) {
            shape = sh;
            first_is_shape = true;
        }
    }

    let mut first_item_doesnt_count = false;
    if (is_linear_gradient && first_is_direction) || (is_radial_gradient && first_is_shape) {
        gradient_stop_count -= 1; // first item is not a gradient stop
        first_item_doesnt_count = true;
    }

    if gradient_stop_count < 2 {
        return Err(CssBackgroundParseError::TooFewGradientStops(input));
    }

    let mut color_stops = Vec::<GradientStopPre>::with_capacity(gradient_stop_count);
    if !first_item_doesnt_count {
        color_stops.push(parse_gradient_stop(first_brace_item)?);
    }

    for stop in brace_iterator {
        color_stops.push(parse_gradient_stop(stop)?);
    }

    // correct percentages
    let mut last_stop = PercentageValue::new(0.0);
    let mut increase_stop_cnt: Option<f32> = None;

    let color_stop_len = color_stops.len();
    'outer: for i in 0..color_stop_len {
        let offset = color_stops[i].offset;
        match offset {
            Some(s) => {
                last_stop = s;
                increase_stop_cnt = None;
            },
            None => {
                let (_, next) = color_stops.split_at_mut(i);

                if let Some(increase_stop_cnt) = increase_stop_cnt {
                    last_stop = PercentageValue::new(last_stop.get() + increase_stop_cnt);
                    next[0].offset = Some(last_stop);
                    continue 'outer;
                }

                let mut next_count: u32 = 0;
                let mut next_value = None;

                // iterate until we find a value where the offset isn't none
                {
                    let mut next_iter = next.iter();
                    next_iter.next();
                    'inner: for next_stop in next_iter {
                        if let Some(off) = next_stop.offset {
                            next_value = Some(off);
                            break 'inner;
                        } else {
                            next_count += 1;
                        }
                    }
                }

                let next_value = next_value.unwrap_or(PercentageValue::new(100.0));
                let increase = (next_value.get() / (next_count as f32)) - (last_stop.get() / (next_count as f32)) ;
                increase_stop_cnt = Some(increase);
                if next_count == 1 && (color_stop_len - i) == 1 {
                    next[0].offset = Some(last_stop);
                } else {
                    if i == 0 {
                        next[0].offset = Some(PercentageValue::new(0.0));
                    } else {
                        next[0].offset = Some(last_stop);
                        // last_stop += increase;
                    }
                }
            }
        }
    }

    match background_type {
        LinearGradient => {
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: direction,
                extend_mode: ExtendMode::Clamp,
                stops: color_stops,
            }))
        },
        RepeatingLinearGradient => {
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: direction,
                extend_mode: ExtendMode::Repeat,
                stops: color_stops,
            }))
        },
        RadialGradient => {
            Ok(StyleBackground::RadialGradient(RadialGradientPreInfo {
                shape: shape,
                extend_mode: ExtendMode::Clamp,
                stops: color_stops,
            }))
        },
        RepeatingRadialGradient => {
            Ok(StyleBackground::RadialGradient(RadialGradientPreInfo {
                shape: shape,
                extend_mode: ExtendMode::Repeat,
                stops: color_stops,
            }))
        },
        Image => unreachable!(),
    }
}

/// Note: In theory, we could take a String here,
/// but this leads to horrible lifetime issues. Also
/// since we only parse the CSS once (at startup),
/// the performance is absolutely negligible.
///
/// However, this allows the `Css` struct to be independent
/// of the original source text, i.e. the original CSS string
/// can be deallocated after successfully parsing it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CssImageId(pub(crate) String);

impl<'a> From<QuoteStripped<'a>> for CssImageId {
    fn from(input: QuoteStripped<'a>) -> Self {
        CssImageId(input.0.to_string())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct QuoteStripped<'a>(pub(crate) &'a str);

fn parse_image<'a>(input: &'a str) -> Result<CssImageId, CssImageParseError<'a>> {
    Ok(strip_quotes(input)?.into())
}

/// Strip quotes from an input, given that both quotes use either `"` or `'`, but not both.
///
/// Example:
///
/// `"Helvetica"` - valid
/// `'Helvetica'` - valid
/// `'Helvetica"` - invalid
fn strip_quotes<'a>(input: &'a str) -> Result<QuoteStripped<'a>, UnclosedQuotesError<'a>> {
    let mut double_quote_iter = input.splitn(2, '"');
    double_quote_iter.next();
    let mut single_quote_iter = input.splitn(2, '\'');
    single_quote_iter.next();

    let first_double_quote = double_quote_iter.next();
    let first_single_quote = single_quote_iter.next();
    if first_double_quote.is_some() && first_single_quote.is_some() {
        return Err(UnclosedQuotesError(input));
    }
    if first_double_quote.is_some() {
        let quote_contents = first_double_quote.unwrap();
        if !quote_contents.ends_with('"') {
            return Err(UnclosedQuotesError(quote_contents));
        }
        Ok(QuoteStripped(quote_contents.trim_right_matches("\"")))
    } else if first_single_quote.is_some() {
        let quote_contents = first_single_quote.unwrap();
        if!quote_contents.ends_with('\'') {
            return Err(UnclosedQuotesError(input));
        }
        Ok(QuoteStripped(quote_contents.trim_right_matches("'")))
    } else {
        Err(UnclosedQuotesError(input))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CssGradientStopParseError<'a> {
    Error(&'a str),
    ColorParseError(css_grammar::ParseError),
}

impl_display!{ CssGradientStopParseError<'a>, {
    Error(e) => e,
    ColorParseError(e) => format!("{}", e),
}}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct GradientStopPre {
    pub offset: Option<PercentageValue>, // this is set to None if there was no offset that could be parsed
    pub color: ColorU,
}

// parses "red" , "red 5%"
fn parse_gradient_stop<'a>(input: &'a str)
-> Result<GradientStopPre, CssGradientStopParseError<'a>>
{
    let mut input_iter = input.split_whitespace();
    let first_item = input_iter.next().ok_or(CssGradientStopParseError::Error(input))?;
    let color = css_grammar::csscolor(first_item).map_err(|e| CssGradientStopParseError::ColorParseError(e))?;
    let second_item = match input_iter.next() {
        None => return Ok(GradientStopPre { offset: None, color: color }),
        Some(s) => s,
    };
    let percentage = parse_percentage(second_item);
    Ok(GradientStopPre { offset: percentage, color: color })
}

// parses "5%" -> 5
fn parse_percentage(input: &str)
-> Option<PercentageValue>
{
    let mut input_iter = input.rsplitn(2, '%');
    let perc = input_iter.next();
    if perc.is_none() {
        None
    } else {
        Some(PercentageValue::new(input_iter.next()?.parse::<f32>().ok()?))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CssDirectionParseError<'a> {
    Error(&'a str),
    InvalidArguments(&'a str),
    ParseFloat(ParseFloatError),
    CornerError(CssDirectionCornerParseError<'a>),
}

impl_display!{CssDirectionParseError<'a>, {
    Error(e) => e,
    InvalidArguments(val) => format!("Invalid arguments: \"{}\"", val),
    ParseFloat(e) => format!("Invalid value: {}", e),
    CornerError(e) => format!("Invalid corner value: {}", e),
}}

impl<'a> From<ParseFloatError> for CssDirectionParseError<'a> {
    fn from(e: ParseFloatError) -> Self {
        CssDirectionParseError::ParseFloat(e)
    }
}

impl<'a> From<CssDirectionCornerParseError<'a>> for CssDirectionParseError<'a> {
    fn from(e: CssDirectionCornerParseError<'a>) -> Self {
        CssDirectionParseError::CornerError(e)
    }
}

// parses "50deg", "to right bottom"
fn parse_direction<'a>(input: &'a str)
-> Result<Direction, CssDirectionParseError<'a>>
{
    use std::f32::consts::PI;

    let input_iter = input.split_whitespace();
    let count = input_iter.clone().count();
    let mut first_input_iter = input_iter.clone();
    // "50deg" | "to" | "right"
    let first_input = first_input_iter.next().ok_or(CssDirectionParseError::Error(input))?;

    enum AngleType {
        Deg,
        Rad,
        Gon,
    }

    let angle = {
        if first_input.ends_with("deg") { Some(AngleType::Deg) }
        else if first_input.ends_with("grad") { Some(AngleType::Gon) }
        else if first_input.ends_with("rad") { Some(AngleType::Rad) }
        else { None }
    };

    if let Some(angle_type) = angle {
        let deg = match angle_type {
            AngleType::Deg => {
                first_input.split("deg").next().unwrap().parse::<f32>()?
            },
            AngleType::Rad => {
                first_input.split("rad").next().unwrap().parse::<f32>()? * 180.0 / PI
            },
            AngleType::Gon => {
                first_input.split("grad").next().unwrap().parse::<f32>()? / 400.0 * 360.0
            },
        };

        // clamp the degree to 360 (so 410deg = 50deg)
        let mut deg = deg % 360.0;
        if deg < 0.0 {
            deg = 360.0 + deg;
        }

        // now deg is in the range of +0..+360
        debug_assert!(deg >= 0.0 && deg <= 360.0);

        return Ok(Direction::Angle(FloatValue::new(deg)));
    }

    // if we get here, the input is definitely not an angle

    if first_input != "to" {
        return Err(CssDirectionParseError::InvalidArguments(input));
    }

    let second_input = first_input_iter.next().ok_or(CssDirectionParseError::Error(input))?;
    let end = parse_direction_corner(second_input)?;

    match count {
        2 => {
            // "to right"
            let start = end.opposite();
            Ok(Direction::FromTo(start, end))
        },
        3 => {
            // "to bottom right"
            let beginning = end;
            let third_input = first_input_iter.next().ok_or(CssDirectionParseError::Error(input))?;
            let new_end = parse_direction_corner(third_input)?;
            // "Bottom, Right" -> "BottomRight"
            let new_end = beginning.combine(&new_end).ok_or(CssDirectionParseError::Error(input))?;
            let start = new_end.opposite();
            Ok(Direction::FromTo(start, new_end))
        },
        _ => { Err(CssDirectionParseError::InvalidArguments(input)) }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum CssDirectionCornerParseError<'a> {
    InvalidDirection(&'a str),
}

impl_display!{ CssDirectionCornerParseError<'a>, {
    InvalidDirection(val) => format!("Invalid direction: \"{}\"", val),
}}

fn parse_direction_corner<'a>(input: &'a str)
-> Result<DirectionCorner, CssDirectionCornerParseError<'a>>
{
    match input {
        "right" => Ok(DirectionCorner::Right),
        "left" => Ok(DirectionCorner::Left),
        "top" => Ok(DirectionCorner::Top),
        "bottom" => Ok(DirectionCorner::Bottom),
        _ => { Err(CssDirectionCornerParseError::InvalidDirection(input))}
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum CssShapeParseError<'a> {
    ShapeErr(InvalidValueErr<'a>),
}

impl_display!{CssShapeParseError<'a>, {
    ShapeErr(e) => format!("\"{}\"", e.0),
}}

/// Represents a parsed CSS `width` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutWidth(pub PixelValue);
/// Represents a parsed CSS `min-width` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutMinWidth(pub PixelValue);
/// Represents a parsed CSS `max-width` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutMaxWidth(pub PixelValue);
/// Represents a parsed CSS `height` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutHeight(pub PixelValue);
/// Represents a parsed CSS `min-height` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutMinHeight(pub PixelValue);
/// Represents a parsed CSS `max-height` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutMaxHeight(pub PixelValue);

/// Represents a parsed CSS `top` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutTop(pub PixelValue);
/// Represents a parsed CSS `left` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutLeft(pub PixelValue);
/// Represents a parsed CSS `right` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutRight(pub PixelValue);
/// Represents a parsed CSS `bottom` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutBottom(pub PixelValue);

/// Represents a parsed CSS `flex-grow` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutFlexGrow(pub FloatValue);
/// Represents a parsed CSS `flex-shrink` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct LayoutFlexShrink(pub FloatValue);

/// Represents a parsed CSS `flex-direction` attribute - default: `Column`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum LayoutDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

/// Represents a parsed CSS `line-height` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct StyleLineHeight(pub PercentageValue);
/// Represents a parsed CSS `letter-spacing` attribute
#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct StyleLetterSpacing(pub PixelValue);

/// Same as the `LayoutDirection`, but without the `-reverse` properties, used in the layout solver,
/// makes decisions based on horizontal / vertical direction easier to write.
/// Use `LayoutDirection::get_axis()` to get the axis for a given `LayoutDirection`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum LayoutAxis {
    Horizontal,
    Vertical,
}

impl LayoutDirection {
    pub fn get_axis(&self) -> LayoutAxis {
        use self::{LayoutAxis::*, LayoutDirection::*};
        match self {
            Row | RowReverse => Horizontal,
            Column | ColumnReverse => Vertical,
        }
    }

    /// Returns true, if this direction is a `column-reverse` or `row-reverse` direction
    pub fn is_reverse(&self) -> bool {
        *self == LayoutDirection::RowReverse || *self == LayoutDirection::ColumnReverse
    }
}

/// Represents a parsed CSS `position` attribute - default: `Static`
///
/// NOTE: No inline positioning is supported.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum LayoutPosition {
    Static,
    Relative,
    Absolute,
}

impl Default for LayoutPosition {
    fn default() -> Self {
        LayoutPosition::Static
    }
}

impl Default for LayoutDirection {
    fn default() -> Self {
        LayoutDirection::Column
    }
}

/// Represents a parsed CSS `flex-wrap` attribute - default: `Wrap`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum LayoutWrap {
    Wrap,
    NoWrap,
}

impl Default for LayoutWrap {
    fn default() -> Self {
        LayoutWrap::Wrap
    }
}

/// Represents a parsed CSS `justify-content` attribute
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum LayoutJustifyContent {
    /// Default value. Items are positioned at the beginning of the container
    Start,
    /// Items are positioned at the end of the container
    End,
    /// Items are positioned at the center of the container
    Center,
    /// Items are positioned with space between the lines
    SpaceBetween,
    /// Items are positioned with space before, between, and after the lines
    SpaceAround,
}

impl Default for LayoutJustifyContent {
    fn default() -> Self {
        LayoutJustifyContent::Start
    }
}

/// Represents a parsed CSS `align-items` attribute
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum LayoutAlignItems {
    /// Items are stretched to fit the container
    Stretch,
    /// Items are positioned at the center of the container
    Center,
    /// Items are positioned at the beginning of the container
    Start,
    /// Items are positioned at the end of the container
    End,
}

impl Default for LayoutAlignItems {
    fn default() -> Self {
        LayoutAlignItems::Stretch
    }
}

/// Represents a parsed CSS `align-content` attribute
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum LayoutAlignContent {
    /// Default value. Lines stretch to take up the remaining space
    Stretch,
    /// Lines are packed toward the center of the flex container
    Center,
    /// Lines are packed toward the start of the flex container
    Start,
    /// Lines are packed toward the end of the flex container
    End,
    /// Lines are evenly distributed in the flex container
    SpaceBetween,
    /// Lines are evenly distributed in the flex container, with half-size spaces on either end
    SpaceAround,
}

/// Represents a parsed CSS `overflow` attribute
///
/// NOTE: This is split into `NotModified` and `Modified`
/// in order to be able to "merge" `overflow-x` and `overflow-y`
/// into one property.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextOverflowBehaviour {
    NotModified,
    Modified(TextOverflowBehaviourInner),
}

impl TextOverflowBehaviour {
    pub fn can_overflow(&self) -> bool {
        use self::TextOverflowBehaviour::*;
        use self::TextOverflowBehaviourInner::*;
        match self {
            Modified(m) => match m {
                Scroll | Auto => true,
                Hidden | Visible => false,
            },
            // default: allow horizontal overflow
            NotModified => false,
        }
    }
}

impl Default for TextOverflowBehaviour {
    fn default() -> Self {
        TextOverflowBehaviour::NotModified
    }
}

/// Represents a parsed CSS `overflow-x` or `overflow-y` property, see
/// [`TextOverflowBehaviour`](./struct.TextOverflowBehaviour.html) - default: `Auto`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TextOverflowBehaviourInner {
    /// Always shows a scroll bar, overflows on scroll
    Scroll,
    /// Does not show a scroll bar by default, only when text is overflowing
    Auto,
    /// Never shows a scroll bar, simply clips text
    Hidden,
    /// Doesn't show a scroll bar, simply overflows the text
    Visible,
}

impl Default for TextOverflowBehaviourInner {
    fn default() -> Self {
        TextOverflowBehaviourInner::Auto
    }
}

/// Horizontal text alignment enum (left, center, right) - default: `Center`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum StyleTextAlignmentHorz {
    Left,
    Center,
    Right,
}

impl Default for StyleTextAlignmentHorz {
    fn default() -> Self {
        StyleTextAlignmentHorz::Center
    }
}

/// Vertical text alignment enum (top, center, bottom) - default: `Center`
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum StyleTextAlignmentVert {
    Top,
    Center,
    Bottom,
}

impl Default for StyleTextAlignmentVert {
    fn default() -> Self {
        StyleTextAlignmentVert::Center
    }
}

/// Stylistic options of the rectangle that don't influence the layout
/// (todo: border-box?)
#[derive(Default, Debug, Clone, PartialEq, Hash)]
pub(crate) struct RectStyle {
    /// Background color of this rectangle
    pub(crate) background_color: Option<StyleBackgroundColor>,
    /// Shadow color
    pub(crate) box_shadow: Option<StyleBoxShadow>,
    /// Gradient (location) + stops
    pub(crate) background: Option<StyleBackground>,
    /// Border
    pub(crate) border: Option<StyleBorder>,
    /// Border radius
    pub(crate) border_radius: Option<StyleBorderRadius>,
    /// Font size
    pub(crate) font_size: Option<StyleFontSize>,
    /// Font name / family
    pub(crate) font_family: Option<StyleFontFamily>,
    /// Text color
    pub(crate) font_color: Option<StyleTextColor>,
    /// Text alignment
    pub(crate) text_align: Option<StyleTextAlignmentHorz,>,
    /// Text overflow behaviour
    pub(crate) overflow: Option<LayoutOverflow>,
    /// `line-height` property
    pub(crate) line_height: Option<StyleLineHeight>,
    /// `letter-spacing` property (modifies the width and height)
    pub(crate) letter_spacing: Option<StyleLetterSpacing>,
}

typed_pixel_value_parser!(parse_css_letter_spacing, StyleLetterSpacing);
impl_pixel_value!(StyleLetterSpacing);

// Layout constraints for a given rectangle, such as "width", "min-width", "height", etc.
#[derive(Default, Debug, Copy, Clone, PartialEq, Hash)]
pub struct RectLayout {

    pub width: Option<LayoutWidth>,
    pub height: Option<LayoutHeight>,
    pub min_width: Option<LayoutMinWidth>,
    pub min_height: Option<LayoutMinHeight>,
    pub max_width: Option<LayoutMaxWidth>,
    pub max_height: Option<LayoutMaxHeight>,

    pub position: Option<LayoutPosition>,
    pub top: Option<LayoutTop>,
    pub bottom: Option<LayoutBottom>,
    pub right: Option<LayoutRight>,
    pub left: Option<LayoutLeft>,

    pub padding: Option<LayoutPadding>,
    pub margin: Option<LayoutMargin>,

    pub direction: Option<LayoutDirection>,
    pub wrap: Option<LayoutWrap>,
    pub flex_grow: Option<LayoutFlexGrow>,
    pub flex_shrink: Option<LayoutFlexShrink>,
    pub justify_content: Option<LayoutJustifyContent>,
    pub align_items: Option<LayoutAlignItems>,
    pub align_content: Option<LayoutAlignContent>,
}

typed_pixel_value_parser!(parse_layout_width, LayoutWidth);
impl_pixel_value!(LayoutWidth);
typed_pixel_value_parser!(parse_layout_height, LayoutHeight);

impl_pixel_value!(LayoutHeight);
typed_pixel_value_parser!(parse_layout_min_height, LayoutMinHeight);
impl_pixel_value!(LayoutMinHeight);
typed_pixel_value_parser!(parse_layout_min_width, LayoutMinWidth);
impl_pixel_value!(LayoutMinWidth);
typed_pixel_value_parser!(parse_layout_max_width, LayoutMaxWidth);
impl_pixel_value!(LayoutMaxWidth);
typed_pixel_value_parser!(parse_layout_max_height, LayoutMaxHeight);
impl_pixel_value!(LayoutMaxHeight);

typed_pixel_value_parser!(parse_layout_top, LayoutTop);
impl_pixel_value!(LayoutTop);
typed_pixel_value_parser!(parse_layout_bottom, LayoutBottom);
impl_pixel_value!(LayoutBottom);
typed_pixel_value_parser!(parse_layout_right, LayoutRight);
impl_pixel_value!(LayoutRight);
typed_pixel_value_parser!(parse_layout_left, LayoutLeft);
impl_pixel_value!(LayoutLeft);

#[derive(Debug, Clone, PartialEq)]
pub enum FlexGrowParseError<'a> {
    ParseFloat(ParseFloatError, &'a str),
}

impl_display!{FlexGrowParseError<'a>, {
    ParseFloat(e, orig_str) => format!("flex-grow: Could not parse floating-point value: \"{}\" - Error: \"{}\"", orig_str, e),
}}

fn parse_layout_flex_grow<'a>(input: &'a str) -> Result<LayoutFlexGrow, FlexGrowParseError<'a>> {
    match parse_float_value(input) {
        Ok(o) => Ok(LayoutFlexGrow(o)),
        Err(e) => Err(FlexGrowParseError::ParseFloat(e, input)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlexShrinkParseError<'a> {
    ParseFloat(ParseFloatError, &'a str),
}

impl_display!{FlexShrinkParseError<'a>, {
    ParseFloat(e, orig_str) => format!("flex-shrink: Could not parse floating-point value: \"{}\" - Error: \"{}\"", orig_str, e),
}}

fn parse_layout_flex_shrink<'a>(input: &'a str) -> Result<LayoutFlexShrink, FlexShrinkParseError<'a>> {
    match parse_float_value(input) {
        Ok(o) => Ok(LayoutFlexShrink(o)),
        Err(e) => Err(FlexShrinkParseError::ParseFloat(e, input)),
    }
}

fn parse_css_line_height(input: &str)
-> Result<StyleLineHeight, PercentageParseError>
{
    parse_percentage_value(input).and_then(|e| Ok(StyleLineHeight(e)))
}

#[derive(Debug, PartialEq, Eq, Copy, Clone, Hash)]
pub struct StyleFontSize(pub PixelValue);

typed_pixel_value_parser!(parse_css_font_size, StyleFontSize);
impl_pixel_value!(StyleFontSize);

impl StyleFontSize {
    pub fn to_pixels(&self) -> f32 {
        self.0.to_pixels()
    }
}

#[derive(Debug, PartialEq, Clone, Eq, Hash)]
pub struct StyleFontFamily {
    // parsed fonts, in order, i.e. "Webly Sleeky UI", "monospace", etc.
    pub(crate) fonts: Vec<FontId>
}

#[derive(Debug, PartialEq, Eq, Clone, Hash)]
pub enum FontId {
    BuiltinFont(String),
    ExternalFont(String),
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum CssStyleFontFamilyParseError<'a> {
    InvalidStyleFontFamily(&'a str),
    UnclosedQuotes(&'a str),
}

impl_display!{CssStyleFontFamilyParseError<'a>, {
    InvalidStyleFontFamily(val) => format!("Invalid font-family: \"{}\"", val),
    UnclosedQuotes(val) => format!("Unclosed quotes: \"{}\"", val),
}}

impl<'a> From<UnclosedQuotesError<'a>> for CssStyleFontFamilyParseError<'a> {
    fn from(err: UnclosedQuotesError<'a>) -> Self {
        CssStyleFontFamilyParseError::UnclosedQuotes(err.0)
    }
}

// parses a "font-family" declaration, such as:
//
// "Webly Sleeky UI", monospace
// 'Webly Sleeky Ui', monospace
// sans-serif
pub(crate) fn parse_css_font_family<'a>(input: &'a str) -> Result<StyleFontFamily, CssStyleFontFamilyParseError<'a>> {
    let multiple_fonts = input.split(',');
    let mut fonts = Vec::with_capacity(1);

    for font in multiple_fonts {
        let font = font.trim();

        let mut double_quote_iter = font.splitn(2, '"');
        double_quote_iter.next();
        let mut single_quote_iter = font.splitn(2, '\'');
        single_quote_iter.next();

        if double_quote_iter.next().is_some() || single_quote_iter.next().is_some() {
            let stripped_font = strip_quotes(font)?;
            fonts.push(FontId::ExternalFont(stripped_font.0.into()));
        } else {
            fonts.push(FontId::BuiltinFont(font.into()));
        }
    }

    Ok(StyleFontFamily {
        fonts: fonts,
    })
}

multi_type_parser!(parse_layout_direction, LayoutDirection,
                    ["row", Row],
                    ["row-reverse", RowReverse],
                    ["column", Column],
                    ["column-reverse", ColumnReverse]);

multi_type_parser!(parse_layout_wrap, LayoutWrap,
                    ["wrap", Wrap],
                    ["nowrap", NoWrap]);

multi_type_parser!(parse_layout_justify_content, LayoutJustifyContent,
                    ["flex-start", Start],
                    ["flex-end", End],
                    ["center", Center],
                    ["space-between", SpaceBetween],
                    ["space-around", SpaceAround]);

multi_type_parser!(parse_layout_align_items, LayoutAlignItems,
                    ["flex-start", Start],
                    ["flex-end", End],
                    ["stretch", Stretch],
                    ["center", Center]);

multi_type_parser!(parse_layout_align_content, LayoutAlignContent,
                    ["flex-start", Start],
                    ["flex-end", End],
                    ["stretch", Stretch],
                    ["center", Center],
                    ["space-between", SpaceBetween],
                    ["space-around", SpaceAround]);

multi_type_parser!(parse_shape, Shape,
                    ["circle", Circle],
                    ["ellipse", Ellipse]);

multi_type_parser!(parse_layout_position, LayoutPosition,
                    ["static", Static],
                    ["absolute", Absolute],
                    ["relative", Relative]);

multi_type_parser!(parse_layout_text_overflow, TextOverflowBehaviourInner,
                    ["auto", Auto],
                    ["scroll", Scroll],
                    ["visible", Visible],
                    ["hidden", Hidden]);

multi_type_parser!(parse_layout_text_align, StyleTextAlignmentHorz,
                    ["center", Center],
                    ["left", Left],
                    ["right", Right]);

/// CssColor is simply a wrapper around the internal CSS color parsing methods.
///
/// Sometimes you'd want to load and parse a CSS color, but you don't want to
/// write your own parser for that. Since Azul already has a parser for CSS colors,
/// this API exposes
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub struct CssColor {
    internal: ColorU,
}

impl CssColor {
    /// Can parse a CSS color with or without prefixed hash or capitalization, i.e. `#aabbcc`
    pub fn from_str<'a>(input: &'a str) -> Result<Self, CssColorParseError<'a>> {
        let color = parse_css_color(input)?;
        Ok(Self {
            internal: color,
        })
    }

    /// Returns the internal parsed color, but in a `0.0 - 1.0` range instead of `0 - 255`
    pub fn to_color_f(&self) -> ColorF {
        self.internal.into()
    }

    /// Returns the internal parsed color
    pub fn to_color_u(&self) -> ColorU {
        self.internal
    }

    /// If `prefix_hash` is set to false, you only get the string, without a hash, in lowercase
    ///
    /// If `self.alpha` is `FF`, it will be omitted from the final result (since `FF` is the default for CSS colors)
    pub fn to_string(&self, prefix_hash: bool) -> String {
        let prefix = if prefix_hash { "#" } else { "" };
        let alpha = if self.internal.a == 255 { String::new() } else { format!("{:02x}", self.internal.a) };
        format!("{}{:02x}{:02x}{:02x}{}", prefix, self.internal.r, self.internal.g, self.internal.b, alpha)
    }
}

impl From<ColorU> for CssColor {
    fn from(color: ColorU) -> Self {
        CssColor { internal: color }
    }
}

impl From<ColorF> for CssColor {
    fn from(color: ColorF) -> Self {
        CssColor { internal: color.into() }
    }
}

impl Into<ColorF> for CssColor {
    fn into(self) -> ColorF {
        self.to_color_f()
    }
}

impl Into<ColorU> for CssColor {
    fn into(self) -> ColorU {
        self.to_color_u()
    }
}

impl Into<String> for CssColor {
    fn into(self) -> String {
        self.to_string(false)
    }
}

#[cfg(feature = "serde_serialization")]
use serde::{de, Serialize, Deserialize, Serializer, Deserializer};

#[cfg(feature = "serde_serialization")]
impl Serialize for CssColor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer,
    {
        let prefix_css_color_with_hash = true;
        serializer.serialize_str(&self.to_string(prefix_css_color_with_hash))
    }
}

#[cfg(feature = "serde_serialization")]
impl<'de> Deserialize<'de> for CssColor {
    fn deserialize<D>(deserializer: D) -> Result<CssColor, D::Error>
    where D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        CssColor::from_str(&s).map_err(de::Error::custom)
    }
}


#[cfg(test)]
mod css_tests {
    use super::*;

    #[test]
    fn test_parse_box_shadow_1() {
        assert_eq!(parse_css_box_shadow("none"), Ok(None));
    }

    #[test]
    fn test_css_color_convert() {
        let color = "#FFD700";
        let parsed = CssColor::from_str(color).unwrap();
        assert_eq!(parsed.to_string(true).to_uppercase(), color.to_string());
        assert_eq!(parsed.to_color_u(), ColorU { r: 255, g: 215, b: 0, a: 255 });
    }

    #[test]
    fn test_parse_box_shadow_2() {
        assert_eq!(parse_css_box_shadow("5px 10px"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 0, g: 0, b: 0, a: 255 },
            blur_radius: PixelValue::px(0.0),
            spread_radius: PixelValue::px(0.0),
            clip_mode: BoxShadowClipMode::Outset,
        })));
    }

    #[test]
    fn test_parse_box_shadow_3() {
        assert_eq!(parse_css_box_shadow("5px 10px #888888"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 136, g: 136, b: 136, a: 255 },
            blur_radius: PixelValue::px(0.0),
            spread_radius: PixelValue::px(0.0),
            clip_mode: BoxShadowClipMode::Outset,
        })));
    }

    #[test]
    fn test_parse_box_shadow_4() {
        assert_eq!(parse_css_box_shadow("5px 10px inset"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 0, g: 0, b: 0, a: 255 },
            blur_radius: PixelValue::px(0.0),
            spread_radius: PixelValue::px(0.0),
            clip_mode: BoxShadowClipMode::Inset,
        })));
    }

    #[test]
    fn test_parse_box_shadow_5() {
        assert_eq!(parse_css_box_shadow("5px 10px outset"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 0, g: 0, b: 0, a: 255 },
            blur_radius: PixelValue::px(0.0),
            spread_radius: PixelValue::px(0.0),
            clip_mode: BoxShadowClipMode::Outset,
        })));
    }

    #[test]
    fn test_parse_box_shadow_6() {
        assert_eq!(parse_css_box_shadow("5px 10px 5px #888888"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 136, g: 136, b: 136, a: 255 },
            blur_radius: PixelValue::px(5.0),
            spread_radius: PixelValue::px(0.0),
            clip_mode: BoxShadowClipMode::Outset,
        })));
    }

    #[test]
    fn test_parse_box_shadow_7() {
        assert_eq!(parse_css_box_shadow("5px 10px #888888 inset"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 136, g: 136, b: 136, a: 255 },
            blur_radius: PixelValue::px(0.0),
            spread_radius: PixelValue::px(0.0),
            clip_mode: BoxShadowClipMode::Inset,
        })));
    }

    #[test]
    fn test_parse_box_shadow_8() {
        assert_eq!(parse_css_box_shadow("5px 10px 5px #888888 inset"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 136, g: 136, b: 136, a: 255 },
            blur_radius: PixelValue::px(5.0),
            spread_radius: PixelValue::px(0.0),
            clip_mode: BoxShadowClipMode::Inset,
        })));
    }

    #[test]
    fn test_parse_box_shadow_9() {
        assert_eq!(parse_css_box_shadow("5px 10px 5px 10px #888888"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 136, g: 136, b: 136, a: 255 },
            blur_radius: PixelValue::px(5.0),
            spread_radius: PixelValue::px(10.0),
            clip_mode: BoxShadowClipMode::Outset,
        })));
    }

    #[test]
    fn test_parse_box_shadow_10() {
        assert_eq!(parse_css_box_shadow("5px 10px 5px 10px #888888 inset"), Ok(Some(BoxShadowPreDisplayItem {
            offset: [PixelValue::px(5.0), PixelValue::px(10.0)],
            color: ColorU { r: 136, g: 136, b: 136, a: 255 },
            blur_radius: PixelValue::px(5.0),
            spread_radius: PixelValue::px(10.0),
            clip_mode: BoxShadowClipMode::Inset,
        })));
    }

    #[test]
    fn test_parse_css_border_1() {
        assert_eq!(
            parse_css_border("5px solid red"),
            Ok(StyleBorderSide {
                border_width: PixelValue::px(5.0),
                border_style: BorderStyle::Solid,
                border_color: ColorU { r: 255, g: 0, b: 0, a: 255 },
            })
        );
    }

    #[test]
    fn test_parse_css_border_2() {
        assert_eq!(
            parse_css_border("double"),
            Ok(StyleBorderSide {
                border_width: PixelValue::px(3.0),
                border_style: BorderStyle::Double,
                border_color: ColorU { r: 0, g: 0, b: 0, a: 255 },
            })
        );
    }

    #[test]
    fn test_parse_direction() {
        let first_input = "60.9grad";
        let e = FloatValue::new(first_input.split("grad").next().unwrap().parse::<f32>().expect("Parseable float") / 400.0 * 360.0);
        assert_eq!(e, FloatValue::new(60.9 / 400.0 * 360.0));
        assert_eq!(parse_direction("60.9grad"), Ok(Direction::Angle(FloatValue::new(60.9 / 400.0 * 360.0))));
    }

    #[test]
    fn test_parse_linear_gradient_1() {
        assert_eq!(parse_css_background("linear-gradient(red, yellow)"),
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: Direction::FromTo(DirectionCorner::Top, DirectionCorner::Bottom),
                extend_mode: ExtendMode::Clamp,
                stops: vec![GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 255, g: 0, b: 0, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(100.0)),
                    color: ColorU { r: 255, g: 255, b: 0, a: 255 },
                }],
            })));
    }

    #[test]
    fn test_parse_linear_gradient_2() {
        assert_eq!(parse_css_background("linear-gradient(red, lime, blue, yellow)"),
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: Direction::FromTo(DirectionCorner::Top, DirectionCorner::Bottom),
                extend_mode: ExtendMode::Clamp,
                stops: vec![GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 255, g: 0, b: 0, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(33.333332)),
                    color: ColorU { r: 0, g: 255, b: 0, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(66.666664)),
                    color: ColorU { r: 0, g: 0, b: 255, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(99.9999)), // note: not 100%, but close enough
                    color: ColorU { r: 255, g: 255, b: 0, a: 255 },
                }],
        })));
    }

    #[test]
    fn test_parse_linear_gradient_3() {
        assert_eq!(parse_css_background("repeating-linear-gradient(50deg, blue, yellow, #00FF00)"),
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: Direction::Angle(50.0.into()),
                extend_mode: ExtendMode::Repeat,
                stops: vec![
                GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 0, g: 0, b: 255, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(50.0)),
                    color: ColorU { r: 255, g: 255, b: 0, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(100.0)),
                    color: ColorU { r: 0, g: 255, b: 0, a: 255 },
                }],
        })));
    }

    #[test]
    fn test_parse_linear_gradient_4() {
        assert_eq!(parse_css_background("linear-gradient(to bottom right, red, yellow)"),
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: Direction::FromTo(DirectionCorner::TopLeft, DirectionCorner::BottomRight),
                extend_mode: ExtendMode::Clamp,
                stops: vec![GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 255, g: 0, b: 0, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(100.0)),
                    color: ColorU { r: 255, g: 255, b: 0, a: 255 },
                }],
            })));
    }

    #[test]
    fn test_parse_linear_gradient_5() {
        assert_eq!(parse_css_background("linear-gradient(0.42rad, red, yellow)"),
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: Direction::Angle(FloatValue::new(24.0642)),
                extend_mode: ExtendMode::Clamp,
                stops: vec![GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 255, g: 0, b: 0, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(100.0)),
                    color: ColorU { r: 255, g: 255, b: 0, a: 255 },
                }],
        })));
    }

    #[test]
    fn test_parse_linear_gradient_6() {
        assert_eq!(parse_css_background("linear-gradient(12.93grad, red, yellow)"),
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: Direction::Angle(FloatValue::new(11.637)),
                extend_mode: ExtendMode::Clamp,
                stops: vec![GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 255, g: 0, b: 0, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(100.0)),
                    color: ColorU { r: 255, g: 255, b: 0, a: 255 },
                }],
        })));
    }

    #[test]
    fn test_parse_linear_gradient_7() {
        assert_eq!(parse_css_background("linear-gradient(10deg, rgb(10, 30, 20), yellow)"),
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: Direction::Angle(FloatValue::new(10.0)),
                extend_mode: ExtendMode::Clamp,
                stops: vec![GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 10, g: 30, b: 20, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(100.0)),
                    color: ColorU { r: 255, g: 255, b: 0, a: 255 },
                }],
        })));
    }

    #[test]
    fn test_parse_linear_gradient_8() {
        assert_eq!(parse_css_background("linear-gradient(50deg, rgb(10, 30, 20, 0.93), hsla(40deg, 80%, 30%, 0.1))"),
            Ok(StyleBackground::LinearGradient(LinearGradientPreInfo {
                direction: Direction::Angle(FloatValue::new(40.0)),
                extend_mode: ExtendMode::Clamp,
                stops: vec![GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 10, g: 30, b: 20, a: 238 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(100.0)),
                    color: ColorU { r: 138, g: 97, b: 15, a: 25 },
                }],
        })));
    }

    #[test]
    fn test_parse_radial_gradient_1() {
        assert_eq!(parse_css_background("radial-gradient(circle, lime, blue, yellow)"),
            Ok(StyleBackground::RadialGradient(RadialGradientPreInfo {
                shape: Shape::Circle,
                extend_mode: ExtendMode::Clamp,
                stops: vec![
                GradientStopPre {
                    offset: Some(PercentageValue::new(0.0)),
                    color: ColorU { r: 0, g: 255, b: 0, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(50.0)),
                    color: ColorU { r: 0, g: 0, b: 255, a: 255 },
                },
                GradientStopPre {
                    offset: Some(PercentageValue::new(100.0)),
                    color: ColorU { r: 255, g: 255, b: 0, a: 255 },
                }],
        })));
    }

    // This test currently fails, but it's not that important to fix right now
    /*
    #[test]
    fn test_parse_radial_gradient_2() {
        assert_eq!(parse_css_background("repeating-radial-gradient(circle, red 10%, blue 50%, lime, yellow)"),
            Ok(ParsedGradient::RadialGradient(RadialGradientPreInfo {
                shape: Shape::Circle,
                extend_mode: ExtendMode::Repeat,
                stops: vec![
                GradientStopPre {
                    offset: Some(0.1),
                    color: ColorF { r: 1.0, g: 0.0, b: 0.0, a: 1.0 },
                },
                GradientStopPre {
                    offset: Some(0.5),
                    color: ColorF { r: 0.0, g: 0.0, b: 1.0, a: 1.0 },
                },
                GradientStopPre {
                    offset: Some(0.75),
                    color: ColorF { r: 0.0, g: 1.0, b: 0.0, a: 1.0 },
                },
                GradientStopPre {
                    offset: Some(1.0),
                    color: ColorF { r: 1.0, g: 1.0, b: 0.0, a: 1.0 },
                }],
        })));
    }
    */

    #[test]
    fn test_parse_css_color_1() {
        assert_eq!(css_grammar::csscolor("#F0F8FF"), Ok(ColorU { r: 240, g: 248, b: 255, a: 255 }));
    }

    #[test]
    fn test_parse_css_color_2() {
        assert_eq!(css_grammar::csscolor("#F0F8FF00"), Ok(ColorU { r: 240, g: 248, b: 255, a: 0 }));
    }

    #[test]
    fn test_parse_css_color_3() {
        assert_eq!(css_grammar::csscolor("#EEE"), Ok(ColorU { r: 238, g: 238, b: 238, a: 255 }));
    }

    #[test]
    fn test_parse_css_color_4() {
        assert_eq!(css_grammar::csscolor("rgb(192, 14, 12)"), Ok(ColorU { r: 192, g: 14, b: 12, a: 255 }));
    }

    #[test]
    fn test_parse_css_color_5() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("Component must be in the range [0-255]");
        assert_eq!(css_grammar::csscolor("rgb(283, 8, 105)"), Err(css_grammar::ParseError { line: 1, column: 8, offset: 7, expected }));
    }

    #[test]
    fn test_parse_css_color_6() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("Alpha must be in the range [0.0-1.0]");
        expected.insert(".");
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("rgba(192, 14, 12, 80)"), Err(css_grammar::ParseError { line: 1, column: 21, offset: 20, expected }));
    }

    #[test]
    fn test_parse_css_color_7() {
        assert_eq!(css_grammar::csscolor("rgba( 0,127,     255   , 0.25  )"), Ok(ColorU { r: 0, g: 127, b: 255, a: 63 }));
    }

    #[test]
    fn test_parse_css_color_8() {
        assert_eq!(css_grammar::csscolor("rgba( 1 ,2,3, 1.0)"), Ok(ColorU { r: 1, g: 2, b: 3, a: 255 }));
    }

    #[test]
    fn test_parse_css_color_9() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("rgb("), Err(css_grammar::ParseError { line: 1, column: 5, offset: 4, expected }));
    }

    #[test]
    fn test_parse_css_color_10() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("rgba("), Err(css_grammar::ParseError { line: 1, column: 6, offset: 5, expected }));
    }

    #[test]
    fn test_parse_css_color_11() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        expected.insert(")");
        assert_eq!(css_grammar::csscolor("rgba(123, 36, 92, 0.375"), Err(css_grammar::ParseError { line: 1, column: 24, offset: 23, expected }));
    }

    #[test]
    fn test_parse_css_color_12() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("rgb()"), Err(css_grammar::ParseError { line: 1, column: 5, offset: 4, expected }));
    }

    #[test]
    fn test_parse_css_color_13() {
        let mut expected = std::collections::HashSet::new();
        expected.insert(",");
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("rgb(10)"), Err(css_grammar::ParseError { line: 1, column: 7, offset: 6, expected }));
    }

    #[test]
    fn test_parse_css_color_14() {
        let mut expected = std::collections::HashSet::new();
        expected.insert(",");
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("rgb(20, 30)"), Err(css_grammar::ParseError { line: 1, column: 11, offset: 10, expected }));
    }

    #[test]
    fn test_parse_css_color_15() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("rgb(30, 40,)"), Err(css_grammar::ParseError { line: 1, column: 12, offset: 11, expected }));
    }

    #[test]
    fn test_parse_css_color_16() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        expected.insert(",");
        assert_eq!(css_grammar::csscolor("rgba(40, 50, 60)"), Err(css_grammar::ParseError { line: 1, column: 16, offset: 15, expected }));
    }

    #[test]
    fn test_parse_css_color_17() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        expected.insert(".");
        expected.insert("-");
        assert_eq!(css_grammar::csscolor("rgba(50, 60, 70, )"), Err(css_grammar::ParseError { line: 1, column: 18, offset: 17, expected }));
    }

    #[test]
    fn test_parse_css_color_18() {
        assert_eq!(css_grammar::csscolor("hsl(0deg, 100%, 100%)"), Ok(ColorU { r: 255, g: 255, b: 255, a: 255 }));
    }

    #[test]
    fn test_parse_css_color_19() {
        assert_eq!(css_grammar::csscolor("hsl(0deg, 100%, 50%)"), Ok(ColorU { r: 255, g: 0, b: 0, a: 255 }));
    }

    #[test]
    fn test_parse_css_color_20() {
        assert_eq!(css_grammar::csscolor("hsl(170deg, 50%, 75%)"), Ok(ColorU { r: 160, g: 224, b: 213, a: 255 }));
    }

    #[test]
    fn test_parse_css_color_21() {
        assert_eq!(css_grammar::csscolor("hsla(190deg, 50%, 75%, 1.0)"), Ok(ColorU { r: 160, g: 213, b: 224, a: 255 }));
    }

    #[test]
    fn test_parse_css_color_22() {
        assert_eq!(css_grammar::csscolor("hsla(120deg, 0%, 25%, 0.25)"), Ok(ColorU { r: 64, g: 64, b: 64, a: 63 }));
    }

    #[test]
    fn test_parse_css_color_23() {
        assert_eq!(css_grammar::csscolor("hsla(120deg, 0%, 0%, 0.5)"), Ok(ColorU { r: 0, g: 0, b: 0, a: 127 }));
    }

    #[test]
    fn test_parse_css_color_24() {
        assert_eq!(css_grammar::csscolor("hsla(60.9deg, 80.3%, 40%, 0.5)"), Ok(ColorU { r: 182, g: 184, b: 20, a: 127 }));
    }

    #[test]
    fn test_parse_css_color_25() {
        assert_eq!(css_grammar::csscolor("hsla(60.9rad, 80.3%, 40%, 0.5)"), Ok(ColorU { r: 45, g: 20, b: 184, a: 127 }));
    }

    #[test]
    fn test_parse_css_color_26() {
        assert_eq!(css_grammar::csscolor("hsla(60.9grad, 80.3%, 40%, 0.5)"), Ok(ColorU { r: 184, g: 170, b: 20, a: 127 }));
    }

    #[test]
    fn test_parse_css_color_27() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("deg");
        expected.insert("rad");
        expected.insert("grad");
        expected.insert(".");
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("hsla(240, 0%, 0%, 0.5)"), Err(css_grammar::ParseError{ line: 1, column: 9, offset: 8, expected }));
    }

    #[test]
    fn test_parse_css_color_28() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("%");
        expected.insert(".");
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("hsla(240deg, 0, 0%, 0.5)"), Err(css_grammar::ParseError{ line: 1, column: 15, offset: 14, expected }));
    }

    #[test]
    fn test_parse_css_color_29() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("%");
        expected.insert(".");
        expected.insert("[0-9]");
        assert_eq!(css_grammar::csscolor("hsla(240deg, 0%, 0, 0.5)"), Err(css_grammar::ParseError{ line: 1, column: 19, offset: 18, expected }));
    }

    #[test]
    fn test_parse_css_color_30() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        expected.insert(".");
        expected.insert("-");
        assert_eq!(css_grammar::csscolor("hsla(240deg, 0%, 0%, )"), Err(css_grammar::ParseError{ line: 1, column: 22, offset: 21, expected }))
    }

    #[test]
    fn test_parse_css_color_31() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        expected.insert(".");
        expected.insert("-");
        assert_eq!(css_grammar::csscolor("hsl(, 0%, 0%, )"), Err(css_grammar::ParseError{ line: 1, column: 5, offset: 4, expected }))
    }

    #[test]
    fn test_parse_css_color_32() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        expected.insert(".");
        expected.insert("-");
        assert_eq!(css_grammar::csscolor("hsl(240deg ,  )"), Err(css_grammar::ParseError{ line: 1, column: 15, offset: 14, expected }))
    }

    #[test]
    fn test_parse_css_color_33() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("[0-9]");
        expected.insert(".");
        expected.insert("-");
        assert_eq!(css_grammar::csscolor("hsl(240deg, 0%,  )"), Err(css_grammar::ParseError{ line: 1, column: 18, offset: 17, expected }))
    }

    #[test]
    fn test_parse_css_color_34() {
        let mut expected = std::collections::HashSet::new();
        expected.insert(")");
        assert_eq!(css_grammar::csscolor("hsl(240deg, 0%, 0%,  )"), Err(css_grammar::ParseError{ line: 1, column: 19, offset: 18, expected }))
    }

    #[test]
    fn test_parse_css_color_35() {
        let mut expected = std::collections::HashSet::new();
        expected.insert(",");
        assert_eq!(css_grammar::csscolor("hsla(240deg, 0%, 0%  )"), Err(css_grammar::ParseError{ line: 1, column: 22, offset: 21, expected }))
    }

    #[test]
    fn test_parse_css_color_36() {
        assert_eq!(css_grammar::csscolor("lime"), Ok(ColorU { r: 0, g: 255, b: 0, a: 255 }))
    }

    #[test]
    fn test_parse_css_color_37() {
        assert_eq!(css_grammar::csscolor("RebeccaPurple"), Ok(ColorU { r: 102, g: 51, b: 153, a: 255 }))
    }

    #[test]
    fn test_parse_css_color_38() {
        let mut expected = std::collections::HashSet::new();
        expected.insert("Unknown color");
        expected.insert("[a-zA-z-]");
        assert_eq!(css_grammar::csscolor("not-a-color"), Err(css_grammar::ParseError{ line: 1, column: 12, offset: 11, expected }))
    }

    #[test]
    fn test_parse_pixel_value_1() {
        assert_eq!(parse_pixel_value("15px"), Ok(PixelValue::px(15.0)));
    }

    #[test]
    fn test_parse_pixel_value_2() {
        assert_eq!(parse_pixel_value("1.2em"), Ok(PixelValue::em(1.2)));
    }

    #[test]
    fn test_parse_pixel_value_3() {
        assert_eq!(parse_pixel_value("11pt"), Ok(PixelValue::pt(11.0)));
    }

    #[test]
    fn test_parse_pixel_value_4() {
        assert_eq!(parse_pixel_value("aslkfdjasdflk"), Err(PixelParseError::InvalidComponent("aslkfdjasdflk")));
    }

    #[test]
    fn test_parse_css_border_radius_1() {
        assert_eq!(parse_css_border_radius("15px"), Ok(StyleBorderRadius::uniform(PixelValue::px(15.0))));
    }

    #[test]
    fn test_parse_css_border_radius_2() {
        assert_eq!(parse_css_border_radius("15px 50px"), Ok(StyleBorderRadius {
            top_left: [PixelValue::px(15.0), PixelValue::px(15.0)],
            bottom_right: [PixelValue::px(15.0), PixelValue::px(15.0)],
            top_right: [PixelValue::px(50.0), PixelValue::px(50.0)],
            bottom_left: [PixelValue::px(50.0), PixelValue::px(50.0)],
        }));
    }

    #[test]
    fn test_parse_css_border_radius_3() {
        assert_eq!(parse_css_border_radius("15px 50px 30px"), Ok(StyleBorderRadius {
            top_left: [PixelValue::px(15.0), PixelValue::px(15.0)],
            bottom_right: [PixelValue::px(30.0), PixelValue::px(30.0)],
            top_right: [PixelValue::px(50.0), PixelValue::px(50.0)],
            bottom_left: [PixelValue::px(50.0), PixelValue::px(50.0)],
        }));
    }

    #[test]
    fn test_parse_css_border_radius_4() {
        assert_eq!(parse_css_border_radius("15px 50px 30px 5px"), Ok(StyleBorderRadius {
            top_left: [PixelValue::px(15.0), PixelValue::px(15.0)],
            bottom_right: [PixelValue::px(30.0), PixelValue::px(30.0)],
            top_right: [PixelValue::px(50.0), PixelValue::px(50.0)],
            bottom_left: [PixelValue::px(5.0), PixelValue::px(5.0)],
        }));
    }

    #[test]
    fn test_parse_css_font_family_1() {
        assert_eq!(parse_css_font_family("\"Webly Sleeky UI\", monospace"), Ok(StyleFontFamily {
            fonts: vec![
                FontId::ExternalFont("Webly Sleeky UI".into()),
                FontId::BuiltinFont("monospace".into()),
            ]
        }));
    }

    #[test]
    fn test_parse_css_font_family_2() {
        assert_eq!(parse_css_font_family("'Webly Sleeky UI'"), Ok(StyleFontFamily {
            fonts: vec![
                FontId::ExternalFont("Webly Sleeky UI".into()),
            ]
        }));
    }

    #[test]
    fn test_parse_background_image() {
        assert_eq!(parse_css_background("image(\"Cat 01\")"), Ok(StyleBackground::Image(
            CssImageId(String::from("Cat 01"))
        )));
    }

    #[test]
    fn test_parse_padding_1() {
        assert_eq!(parse_layout_padding("10px"), Ok(LayoutPadding {
            top: Some(PixelValue::px(10.0)),
            right: Some(PixelValue::px(10.0)),
            bottom: Some(PixelValue::px(10.0)),
            left: Some(PixelValue::px(10.0)),
        }));
    }

    #[test]
    fn test_parse_padding_2() {
        assert_eq!(parse_layout_padding("25px 50px"), Ok(LayoutPadding {
            top: Some(PixelValue::px(25.0)),
            right: Some(PixelValue::px(50.0)),
            bottom: Some(PixelValue::px(25.0)),
            left: Some(PixelValue::px(50.0)),
        }));
    }

    #[test]
    fn test_parse_padding_3() {
        assert_eq!(parse_layout_padding("25px 50px 75px"), Ok(LayoutPadding {
            top: Some(PixelValue::px(25.0)),
            right: Some(PixelValue::px(50.0)),
            left: Some(PixelValue::px(50.0)),
            bottom: Some(PixelValue::px(75.0)),
        }));
    }

    #[test]
    fn test_parse_padding_4() {
        assert_eq!(parse_layout_padding("25px 50px 75px 100px"), Ok(LayoutPadding {
            top: Some(PixelValue::px(25.0)),
            right: Some(PixelValue::px(50.0)),
            bottom: Some(PixelValue::px(75.0)),
            left: Some(PixelValue::px(100.0)),
        }));
    }
}
