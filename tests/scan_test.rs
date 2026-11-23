//! `scan` 模块的集成测试：覆盖 Spring 注解识别、路径取值、行号与注释/字符串过滤。

use openidea_restful_toolkit::scan::{class_base_path, scan_file, HttpMethod};

// 覆盖点 1：没有任何映射注解的普通类 → 扫描结果为空，类级路径为空串。
#[test]
fn no_annotations_yields_nothing() {
    let src = "\
package demo;

public class PlainController {
    public void hello() {
    }
}
";
    assert_eq!(scan_file(src), Vec::new());
    assert_eq!(class_base_path(src), "");
}

// 覆盖点 2：类级 @RequestMapping("/api") + 方法级 @GetMapping("/users") →
// 路径取方法级 /users、方法为 Get、handler 为方法名，行号为 1 基的注解所在行。
#[test]
fn class_and_method_mapping_are_paired() {
    let src = "\
package demo;

@RestController
@RequestMapping(\"/api\")
public class UserController {
    @GetMapping(\"/users\")
    public List<User> listUsers() {
        return null;
    }
}
";
    let endpoints = scan_file(src);
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].http_method, HttpMethod::Get);
    assert_eq!(endpoints[0].path, "/users");
    assert_eq!(endpoints[0].handler, "listUsers");
    // scan_file 的行号按 1 基计，记在 @GetMapping 所在的第 6 行。
    assert_eq!(endpoints[0].line, 6);
    assert_eq!(class_base_path(src), "/api");
}

// 覆盖点 3：@GetMapping 的数组写法 → 每个路径各产出一条（US-E1-01 的笛卡尔积要求）。
#[test]
fn array_mapping_produces_every_path() {
    let src = "\
@RestController
public class AbController {
    @GetMapping({\"/a\", \"/b\"})
    public void both() {
    }
}
";
    let endpoints = scan_file(src);
    assert_eq!(endpoints.len(), 2);
    assert!(endpoints.iter().all(|e| e.http_method == HttpMethod::Get && e.handler == "both"));
    assert_eq!(endpoints[0].path, "/a");
    assert_eq!(endpoints[1].path, "/b");
}

// 覆盖点 4：@GetMapping 无参数。实现要求注解带括号才会进入待定状态，
// 因此 @GetMapping() 产出空串 path；裸写 @GetMapping 不会产生端点，这里一并断言。
#[test]
fn mapping_without_a_path_keeps_an_empty_path() {
    let with_parens = "\
@RestController
public class RootController {
    @GetMapping()
    public void root() {
    }
}
";
    let endpoints = scan_file(with_parens);
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].path, "");

    let bare = "\
@RestController
public class BareController {
    @GetMapping
    public void bare() {
    }
}
";
    assert_eq!(scan_file(bare), Vec::new());
}

// 覆盖点 5：方法级 @RequestMapping(value = "/x", method = RequestMethod.POST)
// → 按 method 属性识别为 Post，而不是笼统的 RequestMapping。
#[test]
fn request_mapping_reads_the_method_attribute() {
    let src = "\
@RestController
public class PostController {
    @RequestMapping(value = \"/x\", method = RequestMethod.POST)
    public void create() {
    }
}
";
    let endpoints = scan_file(src);
    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].http_method, HttpMethod::Post);
    assert_eq!(endpoints[0].path, "/x");
    assert_eq!(endpoints[0].handler, "create");
}

// 覆盖点 6：注释里的假注解（// 行注释、/* */ 块注释）与字符串里的 "@GetMapping"
// 都不应被识别为路由，扫描结果为空。
#[test]
fn fake_annotations_in_comments_and_strings_are_ignored() {
    let src = "\
@RestController
public class FakeController {
    // @GetMapping(\"/fake\")
    /*
     * @PostMapping(\"/also-fake\")
     */
    public void real() {
        String s = \"@GetMapping(/fake)\";
    }
}
";
    assert_eq!(scan_file(src), Vec::new());
}
