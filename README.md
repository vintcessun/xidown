# 一个从xmtv上自动下载戏曲并且上传到bilibili的脚本

为了方便我的阿嫲看戏的体验，我载去年的七八月份写了一个python的脚本（但是运行到现在效率无比的低下并且因为官网上数据越来越多导致解析效率越来越低，于是就重构了这个脚本）

我已经在b上传了很多的关于看戏的视频了，之前有人私信我要exe，但是我是python写的到处很麻烦

刚好最近刚学了rust，用起来有点磕磕巴巴，于是试着重构一个项目试试

That's all.

-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------

2024.7.25更新

之前是写了一个python的版本，但是代码很丑让我不太敢公布，而且错误处理做的一坨，一些也是类型的识别也没有加上，现在这版改了一些特性（比如说只有要下载的时候才会获取mp4地址），代码封装的还可以，基本上可以开盖即食了，然后就是cookie.json，去biliup-rs这个项目里下载release然后登录一下自己的账号就好了，最后程序里的mid是我的，然后自己获取一下就好了，如果要下载的可以模仿一下调用然后下载到本地就可以了
