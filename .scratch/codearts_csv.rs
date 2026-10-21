/// 解析华为云控制台导出的 `credentials.csv`。
///
/// 真实形状（「我的凭证 → 访问密钥 → 新增访问密钥」下载的就是它）：
///
/// ```csv
/// User Name,Access Key Id,Secret Access Key
/// "hid_to9t7v-tyvrj4uz",HPUA2REATHC7Z05QOR2K,s8lJJBnmbauEW7RqsEmIM948poc0VHLQpGzPDcqv
/// ```
///
/// 只有 AK/SK、**没有 STS token** —— 这是合法的长期凭据形态（见 `build_credential`）。
/// 列名中英文都可能出现，所以按表头关键字定位列，而不是写死下标。
///
/// 返回 `(access_key_id, secret_key, user_name)`。
fn parse_credentials_csv(text: &str) -> Option<(String, String, Option<String>)> {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let header = lines.next()?;
    let header_lower = header.to_ascii_lowercase();
    // 只处理"看起来像表头"的输入：必须同时有 access key 与 secret 两列。
    if !(header_lower.contains("access key") && header_lower.contains("secret")) {
        return None;
    }
    let columns: Vec<String> = header
        .split(',')
        .map(|column| column.trim().trim_matches('"').to_ascii_lowercase())
        .collect();
    let find = |needles: &[&str], exclude: &[&str]| -> Option<usize> {
        columns.iter().position(|column| {
            needles.iter().any(|needle| column.contains(needle))
                && !exclude.iter().any(|word| column.contains(word))
        })
    };
    let key_column = find(&["access key"], &[])?;
    let secret_column = find(&["secret"], &[])?;
    // `User Name` 是账号名（可空）；排除掉含 access 的列，免得匹配到 `Access Key Id`。
    let name_column = find(&["user name", "username"], &["access"]);

    let row = lines.next()?;
    let cells = split_csv_row(row);
    let cell = |index: Option<usize>| -> Option<String> {
        index
            .and_then(|index| cells.get(index))
            .map(|value| value.trim().trim_matches('"').to_string())
            .filter(|value| !value.is_empty())
    };
    Some((
        cell(Some(key_column))?,
        cell(Some(secret_column))?,
        cell(name_column),
    ))
}

/// 按逗号切一行 CSV，支持双引号包裹与 `""` 转义。
fn split_csv_row(row: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = row.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                cells.push(current.clone());
                current.clear();
            }
            other => current.push(other),
        }
    }
    cells.push(current);
    cells
}

