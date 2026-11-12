use super::{Currency, Money};

pub fn format_money(m: Money) -> String {
    let sign = if m.cents < 0 { "-" } else { "" };
    let abs = m.cents.unsigned_abs();
    let (whole, frac) = (abs / 100, abs % 100);

    let digits = whole.to_string();
    let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(ch);
    }

    let decimals = if frac == 0 {
        String::new()
    } else {
        format!(",{frac:02}")
    };
    let prefix = match m.currency {
        Currency::Uyu => "$",
        Currency::Usd => "U$S ",
    };
    format!("{sign}{prefix}{grouped}{decimals}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(cents: i64, currency: Currency) -> String {
        format_money(Money { cents, currency })
    }

    #[test]
    fn uyu_whole() {
        assert_eq!(fmt(164_700, Currency::Uyu), "$1.647");
    }

    #[test]
    fn uyu_with_decimals() {
        assert_eq!(fmt(164_750, Currency::Uyu), "$1.647,50");
    }

    #[test]
    fn usd_whole() {
        assert_eq!(fmt(4_900, Currency::Usd), "U$S 49");
    }

    #[test]
    fn usd_grouped_with_decimals() {
        assert_eq!(fmt(129_999, Currency::Usd), "U$S 1.299,99");
    }

    #[test]
    fn zero() {
        assert_eq!(fmt(0, Currency::Uyu), "$0");
    }

    #[test]
    fn pads_single_digit_cents_and_groups_millions() {
        assert_eq!(fmt(100_005, Currency::Uyu), "$1.000,05");
        assert_eq!(fmt(12_345_678_900, Currency::Uyu), "$123.456.789");
    }

    #[test]
    fn negative() {
        assert_eq!(fmt(-164_700, Currency::Uyu), "-$1.647");
    }
}
