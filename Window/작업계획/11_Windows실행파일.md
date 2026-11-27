# Windows 실행 파일 생성과 실행

작업일: 2026-11-26. 상태: Windows x64 release 빌드·실행·정상 종료와 재실행 확인. 6단계 자료 교환과 별도로 사용자 요청에 따라 실행 폴더를 제공한다. 7단계의 installer·서명·다양한 PC 검증은 별도다.

2026-11-27 최신 산출물: `Window/artifacts/StudyApp-0.1.0-windows-x64-20261127T054233353Z/StudyApp.exe`. 메뉴 정리·정답/해설 편집·버튼 통일과 이전 자료 추가/학습 기능을 포함한다. DB 1/2에서 자료를 수정하고 기존 시험 기록은 유지하며 자동 DB 전환은 하지 않는다. 이번 release는 빌드·패키지 해시를 확인했으며 실제 사용자 DB로 실행하지 않았다. [자료 수정 사용 안내](../사용안내/03_정답과해설수정.md)·[이번 검증 범위](20_메뉴와자료편집.md)

## 사용 방법

Window/artifacts 아래 생성된 최신 StudyApp 실행 폴더에서 **StudyApp.exe를 더블클릭**한다. PowerShell·Node·Rust·개발 서버를 켜 둘 필요가 없다. 초기 자료를 읽을 수 있도록 initial-data를 포함한 폴더 전체를 보관하고 이동한다. Windows x64와 Microsoft Edge WebView2 Runtime이 필요하다.

```text
StudyApp-0.1.0-windows-x64-생성시각/
  StudyApp.exe
  initial-data/       catalog와 지정된 학습·보충 파일
  manifest.json      실행 파일과 자료 파일의 SHA-256
  실행안내.txt
```

첫 실행에는 동봉한 자료 2묶음·254문항을 등록한다. 학습 기록은 **%LOCALAPPDATA%/StudyApp/study.db**에 저장한다. 이미 등록된 DB는 계속 사용하며 exe를 다시 실행하거나 새 실행 폴더를 만들어도 초기 자료로 덮어쓰지 않는다. 개발 실행의 .tools/runtime/study.db와 별도이고 기존 개발 기록은 자동 복사하지 않는다. 일반 release 앱에서는 검증용 STUDYAPP_DB_DIR·STUDYAPP_INITIAL_DIR 환경 변수를 적용하지 않는다.

최초 독립 실행 검증 산출물은 Window/artifacts/StudyApp-0.1.0-windows-x64-20261126T021602627Z/StudyApp.exe다. 기존 답변 복원 보완을 포함한 새 산출물은 Window/artifacts/StudyApp-0.1.0-windows-x64-20261126T063733163Z/StudyApp.exe다. 새 빌드와 해시를 확인했으며 사용 중인 기존 창은 종료하지 않았다. 새 버전을 실행하려면 기존 StudyApp을 정상 종료한 뒤 새 exe를 연다. 이전 앱이 실행 중이면 새 exe가 기존 창을 활성화할 수 있다. [보완과 검증 범위](13_기존답변복원보완.md)

이 산출물들은 자료를 포함하므로 Git에서는 제외한다.

## 다시 생성하는 방법

저장소 루트에서 도구·의존성·로컬 data/initial을 준비한 뒤 실행한다.

```powershell
& ./Window/scripts/run-web.ps1 -Task package:exe
```

스크립트 실행 정책에 차단되는 환경에서는 시스템 정책을 변경하지 않고 해당 프로세스에서 실행한다.

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File ./Window/scripts/run-web.ps1 -Task package:exe
```

package-exe.mjs의 준비 검사 → Tauri의 웹 번들·release 빌드 → 실행 폴더 생성 순서다. 출력 경로는 명령 마지막에 표시한다. 원본 catalog가 없거나 지정한 파일이 없으면 실패하며, 남아 있는 개발 자산으로 대신 패키징하지 않는다. catalog가 지정한 파일만 복사해 이전 자산 사본에 남은 파일을 포함하지 않는다. 매번 새로운 시각의 폴더를 만들며 이전 폴더를 지우지 않는다.

## 데이터와 검증 범위

초기 자료를 포함하는 실행 폴더·manifest와 release exe는 기존 ignore의 artifacts·target 아래에 두며 Git에 추가하지 않는다. 원문·변환 결과·채점 예시·기존 DB·사용자 기록은 패키징하지 않는다. exe 실행을 위해 학습 규칙이나 기록 저장 형식은 변경하지 않는다.

## 검증 결과

| 검사 | 결과 |
| --- | --- |
| package:exe | TypeScript·Vite와 Tauri optimized release 빌드·폴더 생성 통과 |
| 파일 구성 | exe·초기 자료 5개·manifest·안내문 총 8개. DB 미포함 |
| 무결성 | manifest 6개 항목의 SHA-256 전부 일치, Windows x64 PE 확인 |
| 독립 실행 | 작업 디렉터리를 임시 폴더로 지정하고 exe 직접 실행. 개발 서버 1420 리스너 없이 화면·저장소 정상 표시 |
| 자료와 DB | 사용자 LocalAppData의 StudyApp/study.db에 2묶음·254문항·프로필 1개·시험 0개, integrity_check 정상 |
| 종료와 재실행 | 창의 정상 종료 후 exe 재실행, 자료·문항·프로필 수 유지와 DB 무결성 정상 |
| 기존 화면 로직 | 11개 테스트 통과. 학습 기능의 Rust 코드는 이번 작업에서 변경하지 않음 |
| 자료 보존 | 원본 53개 내용 유지·Git 제외, artifacts의 exe와 초기 자료도 ignore 확인 |

이번 검증은 이 PC의 WebView2 설치 환경에서 수행했다. 실제 문항이나 답변은 화면에 열거나 시험 기록으로 생성하지 않았다. 다른 PC의 WebView2 미설치 환경, installer·서명·업데이트, 한글 IME 및 앞선 검토의 손상 기록·기존 여러 줄 답변 경계 사례는 미완료 항목으로 유지한다. 이번 작업에서 commit·push는 수행하지 않았다.
