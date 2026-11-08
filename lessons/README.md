# Курсы KTouch

Курсы преобразованы из [us.xml](https://github.com/KDE/ktouch/blob/f0dea77d84c4ddd880074a8eed81e573189ea53b/data/courses/us.xml) и [ru1.xml](https://github.com/KDE/ktouch/blob/f0dea77d84c4ddd880074a8eed81e573189ea53b/data/courses/ru1.xml) репозитория KDE KTouch, версия `f0dea77d84c4ddd880074a8eed81e573189ea53b`. Это US QWERTY и русская раскладка. В метаданных `us.xml` указаны Steinar Theigmann и Håvard Frøiland; `ru1.xml` адаптировал Chemisov Konstantin из ktouch-gen.

Каждый элемент `<lesson>` преобразован в JSON по исходному порядку. Пробельные символы в тексте сведены к одному пробелу; название и новые символы сохранены. ID содержит имя курса и номер урока.

[Руководство KTouch](https://docs.kde.org/stable_kf6/en/ktouch/ktouch/ktouch.pdf) требует GPL версии 2 для данных, включаемых в стандартную поставку. Текст лицензии [GPL-2.0-or-later](GPL-2.0-or-later.txt) взят из репозитория KTouch. В выбранных XML нет отдельных лицензионных заголовков; перед внешней публикацией требуется подтвердить условия именно этих файлов и совместимость с лицензией проекта.
